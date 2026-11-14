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

/// What mt76x2_phy_set_txpower gives the chip on a 5 GHz channel: each
/// chain's power (TX_ALC_CFG_0's CH_INIT fields) and the per-rate offsets
/// from it (TX_PWR_CFG_0, 1, 2, 3, 4, 7, 8 and 9, in that order).
#[derive(Debug, PartialEq, Eq)]
pub struct TxPower {
    pub chain: [u32; 2],
    pub rate_cfg: [u32; 8],
}

/// mt76x2_phy_set_txpower for 5 GHz, with no user limit. The EEPROM gives
/// each rate's power relative to the target (faster rates lower: their
/// denser modulation distorts in an amplifier driven as hard as the slow
/// rates'); the chains are set to the lowest of those and every rate is
/// offset up from it.
pub fn tx_power_5g(eeprom: &[u8], channel: u8, bw40: bool) -> TxPower {
    use crate::regs::ee;
    let ee16 = |at: usize| match eeprom.get(at..at + 2) {
        Some(b) => u16::from_le_bytes([b[0], b[1]]),
        None => 0xffff,
    };
    let byte = |at: usize| eeprom.get(at).copied().unwrap_or(0xff);

    // mt76x2_get_power_info
    let group = cal_group_5g(channel);
    let delta_idx = power_delta_index_5g(channel);
    let chains = [ee::TX_POWER_0_START_5G, ee::TX_POWER_1_START_5G].map(|start| {
        let at = start + group * 5;
        (
            i32::from(byte(at + 2) as i8),
            sign_extend_optional(byte(at + delta_idx), 7),
        )
    });
    let mut target = i32::from(ee16(ee::RF_2G_RX_HIGH_GAIN) & 0xff);
    if tssi_enabled(eeprom) || target == 0 || target == 0xff {
        target = chains[0].0;
    }
    let delta_bw40 = rate_power_val((ee16(ee::TX_POWER_DELTA_BW40) >> 8) as u8);

    // mt76x2_get_rate_power: cck[4], ofdm[8], ht[16], vht[2]
    let mut t = [0i32; 30];
    let pair = |t: &mut [i32; 30], i: usize, val: u16| {
        t[i] = rate_power_val(val as u8);
        t[i + 1] = t[i];
        t[i + 2] = rate_power_val((val >> 8) as u8);
        t[i + 3] = t[i + 2];
    };
    pair(&mut t, 0, ee16(ee::TX_POWER_CCK));
    pair(&mut t, 4, ee16(ee::TX_POWER_OFDM_5G_6M));
    pair(&mut t, 8, ee16(ee::TX_POWER_OFDM_5G_24M));
    pair(&mut t, 12, ee16(ee::TX_POWER_HT_MCS0));
    pair(&mut t, 16, ee16(ee::TX_POWER_HT_MCS4));
    pair(&mut t, 20, ee16(ee::TX_POWER_HT_MCS8));
    pair(&mut t, 24, ee16(ee::TX_POWER_HT_MCS12));
    t[28] = rate_power_val((ee16(ee::TX_POWER_VHT_MCS8) >> 8) as u8);
    t[29] = t[28];

    // mt76x2_phy_set_txpower
    let delta = if bw40 { delta_bw40 } else { 0 };
    t.iter_mut()
        .for_each(|r| *r = i32::from((*r + target + delta) as i8));
    let mut base = t.iter().copied().filter(|&r| r != 0).min().unwrap_or(0);
    let delta = base - target;
    let mut txp = chains.map(|(power, d)| power + d + delta);
    let gain = txp[0].min(txp[1]);
    if gain < 0 {
        base -= gain;
        txp = txp.map(|p| p - gain);
    } else if gain > 0x2f {
        base -= gain - 0x2f;
        txp = [0x2f; 2];
    }
    t.iter_mut().for_each(|r| *r -= base);

    // mt76x02_phy_set_txpower
    let mask = |v: [i32; 4]| {
        v.iter()
            .enumerate()
            .fold(0u32, |acc, (i, &x)| acc | ((x as u32) & 0x3f) << (8 * i))
    };
    let (cck, ofdm, ht, vht) = (&t[0..4], &t[4..12], &t[12..28], &t[28..30]);
    TxPower {
        chain: txp.map(|p| (p & 0x3f) as u32),
        rate_cfg: [
            mask([cck[0], cck[2], ofdm[0], ofdm[2]]),
            mask([ofdm[4], ofdm[6], ht[0], ht[2]]),
            mask([ht[4], ht[6], ht[8], ht[10]]),
            mask([ht[12], ht[14], ht[0], ht[2]]),
            mask([ht[4], ht[6], 0, 0]),
            mask([ofdm[7], vht[0], ht[7], vht[1]]),
            mask([ht[14], 0, vht[0], vht[1]]),
            mask([ht[7], 0, vht[0], vht[1]]),
        ],
    }
}

/// mt76x2_tssi_enabled: the chip measures its output power (TSSI) and
/// corrects it, rather than by temperature alone.
pub fn tssi_enabled(eeprom: &[u8]) -> bool {
    use crate::regs::ee;
    let ee16 = |at: usize| match eeprom.get(at..at + 2) {
        Some(b) => u16::from_le_bytes([b[0], b[1]]),
        None => 0xffff,
    };
    let conf1 = ee16(ee::NIC_CONF_1);
    let temp_alc =
        ee16(ee::TX_POWER_EXT_PA_5G) & (1 << 15) != 0 && conf1 & ee::NIC_CONF_1_TEMP_TX_ALC != 0;
    !temp_alc && conf1 & ee::NIC_CONF_1_TX_ALC_EN != 0
}

/// Each chain's TSSI slope and offset for a 5 GHz channel, the first two
/// bytes of its power group (mt76x2_get_power_info_5g).
pub fn tssi_slope_offset_5g(eeprom: &[u8], channel: u8) -> ([u8; 2], [u8; 2]) {
    use crate::regs::ee;
    let group = cal_group_5g(channel);
    let at = |start: usize, i: usize| eeprom.get(start + group * 5 + i).copied().unwrap_or(0);
    (
        [
            at(ee::TX_POWER_0_START_5G, 0),
            at(ee::TX_POWER_1_START_5G, 0),
        ],
        [
            at(ee::TX_POWER_0_START_5G, 1),
            at(ee::TX_POWER_1_START_5G, 1),
        ],
    )
}

/// The byte of a 5 GHz power group that holds the channel's delta
/// (mt76x2_get_power_info_5g).
fn power_delta_index_5g(channel: u8) -> usize {
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
    if channel >= 192 {
        4
    } else if channel >= 184 {
        3
    } else {
        BELOW
            .iter()
            .find(|(limit, _)| channel < *limit)
            .map_or(4, |&(_, idx)| idx)
    }
}

/// mt76x02_rate_power_val: an EEPROM rate power byte, 0 when unset.
fn rate_power_val(val: u8) -> i32 {
    if val == 0 || val == 0xff {
        0
    } else {
        sign_extend_optional(val, 7)
    }
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
        assert_eq!(tx_power_5g(&eeprom, 149, false).chain, [0x12, 0x0f]);
        // no delta when bit 7 is clear
        eeprom[0x062 + 25 + 3] = 0x7f;
        assert_eq!(tx_power_5g(&eeprom, 149, false).chain, [0x10, 0x0f]);
    }

    #[test]
    fn faster_rates_go_out_at_their_lower_power() {
        let mut eeprom = vec![0xffu8; 512];
        // NIC_CONF_1: no TSSI; target power 0x20; no 40 MHz delta
        eeprom[0x036..0x038].copy_from_slice(&0u16.to_le_bytes());
        eeprom[0x0f8] = 0x20;
        eeprom[0x051] = 0;
        // chain 0 and 1, group 5: 0x20, no channel delta
        for start in [0x062, 0x080] {
            eeprom[start + 25 + 2] = 0x20;
            eeprom[start + 25 + 3] = 0;
        }
        // OFDM 6/9 and 12/18 at 0; 24/36 at -1 dB; 48/54 at -3 dB
        eeprom[0x0b2..0x0b4].copy_from_slice(&[0x80, 0x80]);
        eeprom[0x0b4..0x0b6].copy_from_slice(&[0x80 | 2, 0x80 | 6]);
        // MCS 0-3 at 0; MCS 4/5 at -2 dB; MCS 6/7 at -4 dB
        eeprom[0x0a6..0x0a8].copy_from_slice(&[0x80, 0x80]);
        eeprom[0x0a8..0x0aa].copy_from_slice(&[0x80 | 4, 0x80 | 8]);
        let p = tx_power_5g(&eeprom, 149, true);
        // The chains at the lowest rate's power: MCS 6/7, 4 dB down.
        assert_eq!(p.chain, [0x20 - 8; 2]);
        // 6 Mbit/s 8 half-dB above that, 54 Mbit/s 2, MCS 4/5 4, MCS 6/7 0.
        assert_eq!(p.rate_cfg[0] >> 16 & 0x3f, 8);
        assert_eq!(p.rate_cfg[1] >> 8 & 0x3f, 2);
        assert_eq!(p.rate_cfg[2] & 0x3f, 4);
        assert_eq!(p.rate_cfg[2] >> 8 & 0x3f, 0);
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
