//! An in-memory radio medium for `Node`s: one simulated clock, frames heard
//! by every other node on the same channel when they're sent, a little
//! airtime, and a share of them lost.

#![allow(dead_code)]

use owly_awdl::frame::Frame;
use owly_awdl::{Action, Channel, Config, Input, Ipv6Packet, MacAddr, Node, PeerInfo, Time};

pub const AIRTIME_US: u64 = 200;

pub struct SimNode {
    pub node: Node,
    pub timer: Time,
    pub channel: Channel,
    pub delivered: Vec<(Time, Ipv6Packet)>,
    pub up: Vec<(Time, PeerInfo)>,
    pub down: Vec<(Time, MacAddr)>,
    pub sent: u64,
    /// Every action, in order, for tests that check ordering.
    pub log: Vec<Action>,
}

struct InFlight {
    from: usize,
    at: Time,
    channel: Channel,
    frame: Frame,
}

pub struct Sim {
    pub now: Time,
    pub nodes: Vec<SimNode>,
    flight: Vec<InFlight>,
    /// Out of 1000 frames, how many each receiver misses.
    pub loss_permille: u32,
    rng: u64,
}

impl Sim {
    pub fn new(configs: Vec<Config>, loss_permille: u32, seed: u64) -> Sim {
        let now = Time(1_000_000);
        let mut sim = Sim {
            now,
            nodes: Vec::new(),
            flight: Vec::new(),
            loss_permille,
            rng: seed | 1,
        };
        for (i, cfg) in configs.into_iter().enumerate() {
            let channel = cfg.radio.channel;
            // Nodes start at different phases, as real ones do.
            let start = now.plus(i as u64 * 17_321);
            let node = Node::new(cfg, start);
            sim.nodes.push(SimNode {
                node,
                timer: start,
                channel,
                delivered: Vec::new(),
                up: Vec::new(),
                down: Vec::new(),
                sent: 0,
                log: Vec::new(),
            });
        }
        sim
    }

    fn random(&mut self) -> u32 {
        // xorshift64
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x % 1000) as u32
    }

    pub fn input(&mut self, i: usize, input: Input) {
        let actions: Vec<Action> = self.nodes[i].node.handle(input).collect();
        self.apply(i, actions);
    }

    fn apply(&mut self, i: usize, actions: Vec<Action>) {
        for a in actions {
            let n = &mut self.nodes[i];
            n.log.push(a.clone());
            match a {
                Action::Transmit { at, frame } => {
                    n.sent += 1;
                    let channel = n.channel;
                    self.flight.push(InFlight {
                        from: i,
                        at: at.plus(AIRTIME_US),
                        channel,
                        frame,
                    });
                }
                Action::ArmTimer(t) => n.timer = t,
                Action::PeerUp(p) => n.up.push((self.now, p)),
                Action::PeerDown(m) => n.down.push((self.now, m)),
                Action::Deliver(p) => n.delivered.push((self.now, p)),
            }
        }
    }

    /// Runs until `until`, or until `done` holds after an event.
    pub fn run_until(&mut self, until: Time, mut done: impl FnMut(&Sim) -> bool) -> bool {
        loop {
            if done(self) {
                return true;
            }
            let next_timer = self
                .nodes
                .iter()
                .enumerate()
                .min_by_key(|(_, n)| n.timer)
                .map(|(i, n)| (i, n.timer));
            let next_frame = self
                .flight
                .iter()
                .enumerate()
                .min_by_key(|(_, f)| f.at)
                .map(|(i, f)| (i, f.at));
            let (is_frame, idx, at) = match (next_timer, next_frame) {
                (Some((_, tt)), Some((fi, ft))) if ft <= tt => (true, fi, ft),
                (Some((ti, tt)), _) => (false, ti, tt),
                (None, Some((fi, ft))) => (true, fi, ft),
                (None, None) => return false,
            };
            if at > until {
                self.now = until;
                return done(self);
            }
            self.now = at.max(self.now);
            if is_frame {
                let f = self.flight.swap_remove(idx);
                for j in 0..self.nodes.len() {
                    if j == f.from || self.nodes[j].channel != f.channel {
                        continue;
                    }
                    if self.random() < self.loss_permille {
                        continue;
                    }
                    let now = self.now;
                    let bytes = f.frame.bytes.clone();
                    self.input(
                        j,
                        Input::Frame {
                            rx_time: now,
                            rssi: Some(-40),
                            bytes: &bytes,
                        },
                    );
                }
            } else {
                let now = self.now;
                self.input(idx, Input::Timer(now));
            }
        }
    }

    pub fn run_for(&mut self, us: u64) {
        let until = self.now.plus(us);
        self.run_until(until, |_| false);
    }

    /// Has the IP stack on node `from` send `data` to node `to`, link-local
    /// to link-local.
    pub fn send(&mut self, from: usize, to: usize, payload: &[u8]) {
        let src = self.nodes[from].node.config().addr;
        let dst = self.nodes[to].node.config().addr;
        let packet = Ipv6Packet::echo_request(src, dst, 0x1234, 1, payload);
        let now = self.now;
        self.input(from, Input::Outbound { now, packet });
    }

    pub fn valid(&self, i: usize, peer: usize) -> bool {
        let mac = self.nodes[peer].node.config().addr;
        self.nodes[i].node.peer(mac).is_some_and(|p| p.valid)
    }

    /// Every node follows the same master.
    pub fn one_master(&self) -> Option<MacAddr> {
        let m = self.nodes[0].node.election().master;
        self.nodes
            .iter()
            .all(|n| n.node.election().master == m)
            .then_some(m)
    }

    pub fn got(&self, i: usize, payload: &[u8]) -> bool {
        self.nodes[i]
            .delivered
            .iter()
            .any(|(_, p)| p.data.ends_with(payload))
    }
}

pub fn mac(n: u8) -> MacAddr {
    MacAddr([0x02, 0, 0, 0, 0, n])
}
