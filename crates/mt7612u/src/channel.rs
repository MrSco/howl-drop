//! A channel and what mt76x2u_phy_set_channel derives from it.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Ht20,
    /// 40 MHz, the secondary channel above (HT40+).
    Ht40Plus,
    /// 40 MHz, the secondary channel below (HT40-).
    Ht40Minus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channel {
    /// The primary channel's number: 6, 44, 149.
    pub primary: u8,
    pub width: Width,
}

/// What the registers and the MCU's SWITCH_CHANNEL take for a channel.
#[derive(Debug, PartialEq, Eq)]
pub struct Tuning {
    /// The centre channel the MCU tunes to (151 for 149 HT40+).
    pub center: u8,
    /// 0: 20 MHz, 1: 40 MHz.
    pub bw: u8,
    /// Where the primary is in the 40 MHz (0xe0 + this is ext_chan).
    pub bw_index: u8,
    /// 0 for the lower 20 MHz, 1 for the upper.
    pub group: u8,
}

impl Channel {
    pub fn new(primary: u8, width: Width) -> Self {
        Channel { primary, width }
    }

    pub fn is_5ghz(&self) -> bool {
        self.primary > 14
    }

    pub fn tuning(&self) -> Tuning {
        match self.width {
            Width::Ht20 => Tuning {
                center: self.primary,
                bw: 0,
                bw_index: 0,
                group: 0,
            },
            Width::Ht40Plus => Tuning {
                center: self.primary + 2,
                bw: 1,
                bw_index: 1,
                group: 0,
            },
            Width::Ht40Minus => Tuning {
                center: self.primary - 2,
                bw: 1,
                bw_index: 3,
                group: 1,
            },
        }
    }

    /// EXT_CCA_CFG's CCA fields for the primary's place (ext_cca_chan).
    pub fn ext_cca(&self) -> u32 {
        // CCA0..3 two bits each, then the CCA mask
        const BY_GROUP: [u32; 2] = [
            (1 << 2) | (2 << 4) | (3 << 6) | (1 << 8),
            1 | (2 << 4) | (3 << 6) | (1 << 9),
        ];
        BY_GROUP[self.tuning().group as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_149_at_40_mhz_tunes_to_151() {
        let t = Channel::new(149, Width::Ht40Plus).tuning();
        assert_eq!(
            t,
            Tuning {
                center: 151,
                bw: 1,
                bw_index: 1,
                group: 0
            }
        );
        let t = Channel::new(153, Width::Ht40Minus).tuning();
        assert_eq!(
            t,
            Tuning {
                center: 151,
                bw: 1,
                bw_index: 3,
                group: 1
            }
        );
        assert_eq!(Channel::new(6, Width::Ht20).tuning().center, 6);
        assert!(!Channel::new(6, Width::Ht20).is_5ghz());
    }

    #[test]
    fn ext_cca_is_as_mt76_lists_it() {
        // ext_cca_chan[0]: CCA0 0, CCA1 1, CCA2 2, CCA3 3, mask BIT(0)
        assert_eq!(Channel::new(149, Width::Ht40Plus).ext_cca(), 0x1e4);
        // ext_cca_chan[1]: CCA0 1, CCA1 0, CCA2 2, CCA3 3, mask BIT(1)
        assert_eq!(Channel::new(153, Width::Ht40Minus).ext_cca(), 0x2e1);
    }
}
