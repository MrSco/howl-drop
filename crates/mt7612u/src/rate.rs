//! The rate for unicast data, chosen from how the air is going.
//!
//! mt76x2u leaves rate control to mac80211's minstrel, which needs each
//! frame's TX status; here only the chip's counters are read (TX_STA_CNT0
//! and 1: frames given up, and retries), once a second. That is enough for
//! a link to one iPhone at a time: a step down when frames start needing
//! retries, a step up while they don't, and a pause after an upward step
//! that went badly, so a link at the edge doesn't keep trying the rate
//! above it.

use std::time::{Duration, Instant};

use crate::tx::Rate;

/// The rates tried: 12, 24, 36 and 54 Mbit/s.
pub const LEGACY: &[Rate] = &[
    Rate::Legacy(24),
    Rate::Legacy(48),
    Rate::Legacy(72),
    Rate::Legacy(108),
];

/// Those, then HT across 40 MHz: MCS 4, 5 and 7 on one stream (81, 108
/// and 135 Mbit/s), then 12, 13 and 15 on two (162, 216, 270 Mbit/s).
pub const WITH_HT40: &[Rate] = &[
    Rate::Legacy(24),
    Rate::Legacy(48),
    Rate::Legacy(72),
    Rate::Legacy(108),
    Rate::Ht40 { mcs: 4, sgi: false },
    Rate::Ht40 { mcs: 5, sgi: false },
    Rate::Ht40 { mcs: 7, sgi: false },
    Rate::Ht40 {
        mcs: 12,
        sgi: false,
    },
    Rate::Ht40 {
        mcs: 13,
        sgi: false,
    },
    Rate::Ht40 {
        mcs: 15,
        sgi: false,
    },
];

/// Fewer unicast data frames than this in a second say nothing.
const MIN_FRAMES: u64 = 30;
/// Retries per frame above which the rate steps down; a clean link at any
/// of the ladder's rates measured 0.2-0.3, a failing one about 2.
const DOWN_RETRIES: f64 = 1.0;
/// Retries per frame below which the rate steps up.
const UP_RETRIES: f64 = 0.5;
/// Frames given up after every retry, as a share, above which it steps down.
const DOWN_FAILED: f64 = 0.05;
/// How long a rate that failed after an upward step isn't tried again.
const HOLD: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct AutoRate {
    ladder: &'static [Rate],
    idx: usize,
    /// No step up before this.
    hold_until: Instant,
    /// Whether the last change was a step up, still being judged.
    probing: bool,
}

impl AutoRate {
    /// Starting at 24 Mbit/s: quick to step either way, and safer than
    /// 54 for the first frames of a link at range.
    pub fn new(now: Instant, ladder: &'static [Rate]) -> AutoRate {
        AutoRate {
            ladder,
            idx: 1,
            hold_until: now,
            probing: false,
        }
    }

    /// The rate now.
    pub fn rate(&self) -> Rate {
        self.ladder[self.idx]
    }

    /// One second's count: unicast data frames sent, and of all frames sent
    /// those given up and the retries. The new rate, if it changed.
    pub fn step(&mut self, now: Instant, frames: u64, failed: u64, retries: u64) -> Option<Rate> {
        if frames < MIN_FRAMES {
            return None;
        }
        let per_frame = retries as f64 / frames as f64;
        let failed = failed as f64 / frames as f64;
        let probing = std::mem::take(&mut self.probing);
        if per_frame > DOWN_RETRIES || failed > DOWN_FAILED {
            if probing {
                self.hold_until = now + HOLD;
            }
            if self.idx == 0 {
                return None;
            }
            self.idx -= 1;
            return Some(self.rate());
        }
        if per_frame < UP_RETRIES
            && failed < DOWN_FAILED / 2.0
            && now >= self.hold_until
            && self.idx + 1 < self.ladder.len()
        {
            self.idx += 1;
            self.probing = true;
            return Some(self.rate());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_link_climbs_to_54_and_stays() {
        let t = Instant::now();
        let mut r = AutoRate::new(t, LEGACY);
        assert_eq!(r.rate(), Rate::Legacy(48));
        assert_eq!(r.step(t, 500, 0, 100), Some(Rate::Legacy(72)));
        assert_eq!(r.step(t, 500, 0, 100), Some(Rate::Legacy(108)));
        assert_eq!(r.step(t, 500, 0, 100), None);
        assert_eq!(r.rate(), Rate::Legacy(108));
    }

    #[test]
    fn retries_step_it_down_and_a_failed_climb_waits() {
        let t = Instant::now();
        let mut r = AutoRate::new(t, LEGACY);
        // 24 -> 36 fails: back to 24, and 36 isn't tried for a while.
        assert_eq!(r.step(t, 500, 0, 100), Some(Rate::Legacy(72)));
        assert_eq!(r.step(t, 500, 10, 1000), Some(Rate::Legacy(48)));
        assert_eq!(r.step(t + Duration::from_secs(1), 500, 0, 100), None);
        assert_eq!(
            r.step(t + HOLD + Duration::from_secs(1), 500, 0, 100),
            Some(Rate::Legacy(72))
        );
    }

    #[test]
    fn frames_given_up_step_it_down_to_12_at_most() {
        let t = Instant::now();
        let mut r = AutoRate::new(t, LEGACY);
        assert_eq!(r.step(t, 500, 50, 200), Some(Rate::Legacy(24)));
        assert_eq!(r.step(t, 500, 50, 200), None);
        assert_eq!(r.rate(), Rate::Legacy(24));
    }

    #[test]
    fn with_ht40_a_clean_link_climbs_past_54_to_two_streams() {
        let t = Instant::now();
        let mut r = AutoRate::new(t, WITH_HT40);
        while r.step(t, 500, 0, 100).is_some() {}
        assert_eq!(
            r.rate(),
            Rate::Ht40 {
                mcs: 15,
                sgi: false
            }
        );
    }

    #[test]
    fn a_quiet_second_changes_nothing() {
        let t = Instant::now();
        let mut r = AutoRate::new(t, LEGACY);
        assert_eq!(r.step(t, 5, 5, 50), None);
        assert_eq!(r.rate(), Rate::Legacy(48));
    }
}
