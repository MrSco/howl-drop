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

/// mt76x2_get_cal_channel_group: the 5 GHz calibration group a channel's
/// EEPROM values are kept under (Japan, UNII-1, -2, -2e low, -2e high, -3).
pub fn cal_group_5g(channel: u8) -> usize {
    if (184..=196).contains(&channel) {
        0
    } else if channel <= 48 {
        1
    } else if channel <= 64 {
        2
    } else if channel <= 114 {
        3
    } else if channel <= 144 {
        4
    } else {
        5
    }
}

/// Each chain's transmit power on a 5 GHz channel, as mt76x2's 0x3f
/// CH_INIT field takes it: the EEPROM's target power for the channel's group
/// plus its delta for where the channel is in the group
/// (mt76x2_get_power_info_5g), within 0 to 0x2f.
pub fn chain_power_5g(eeprom: &[u8], channel: u8) -> [u32; 2] {
    use crate::regs::ee;
    // The first of these the channel is below gives the delta's byte; from
    // 184 the Japanese channels, as mt76x2_get_power_info_5g orders them.
    const BELOW: [(u8, usize); 9] = [
        (44, 3),
        (52, 4),
        (58, 3),
        (98, 4),
        (106, 3),
        (116, 4),
        (130, 3),
        (149, 4),
        (157, 3),
    ];
    let delta_idx = if channel >= 192 {
        4
    } else if channel >= 184 {
        3
    } else {
        BELOW
            .iter()
            .find(|(limit, _)| channel < *limit)
            .map_or(4, |&(_, idx)| idx)
    };
    let group = cal_group_5g(channel);
    [ee::TX_POWER_0_START_5G, ee::TX_POWER_1_START_5G].map(|start| {
        let at = start + group * 5;
        let byte = |i: usize| eeprom.get(at + i).copied().unwrap_or(0);
        let power = i32::from(byte(2)) + sign_extend_optional(byte(delta_idx), 7);
        power.clamp(0, 0x2f) as u32
    })
}

/// mt76x02_sign_extend_optional: bit `size` says whether the value is there;
/// then bit `size - 1` set means positive, and the bits below are the size.
fn sign_extend_optional(val: u8, size: u32) -> i32 {
    let val = u32::from(val);
    if val & (1 << size) == 0 {
        return 0;
    }
    let magnitude = (val & ((1 << (size - 1)) - 1)) as i32;
    if val & (1 << (size - 1)) != 0 {
        magnitude
    } else {
        -magnitude
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_149_takes_unii_3s_power() {
        assert_eq!(cal_group_5g(149), 5);
        assert_eq!(cal_group_5g(44), 1);
        let mut eeprom = vec![0u8; 512];
        // chain 0, group 5: target 0x10, delta (index 3) +2
        eeprom[0x062 + 25 + 2] = 0x10;
        eeprom[0x062 + 25 + 3] = 0x80 | 0x40 | 2;
        // chain 1, group 5: target 0x12, delta -3
        eeprom[0x080 + 25 + 2] = 0x12;
        eeprom[0x080 + 25 + 3] = 0x80 | 3;
        assert_eq!(chain_power_5g(&eeprom, 149), [0x12, 0x0f]);
        // no delta when bit 7 is clear, and never above 0x2f
        eeprom[0x062 + 25 + 3] = 0x7f;
        eeprom[0x080 + 25 + 2] = 0x40;
        assert_eq!(chain_power_5g(&eeprom, 149), [0x10, 0x2f]);
    }

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
