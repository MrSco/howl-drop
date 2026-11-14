//! Register addresses and bits, named as in mt76 (mt76x02_regs.h,
//! mt76x02_mcu.h, mt76x2/mcu.h). The CFG ones are in the USB controller's
//! space ([`crate::Space::Cfg`]), the rest in the MAC's.

/// The chip's ID and revision (MT_ASIC_VERSION): 0x7612 in the high half.
pub const ASIC_VERSION: u32 = 0x0000;
/// Reads 0 or all ones until the MAC is up (mt76x02_wait_for_mac).
pub const MAC_CSR0: u32 = 0x1000;

pub const WLAN_FUN_CTRL: u32 = 0x0080;
pub const WLAN_FUN_CTRL_WLAN_EN: u32 = 1 << 0;
pub const WLAN_FUN_CTRL_WLAN_CLK_EN: u32 = 1 << 1;
pub const WLAN_FUN_CTRL_WLAN_RESET_RF: u32 = 1 << 2;
pub const WLAN_FUN_CTRL_FRC_WL_ANT_SEL: u32 = 1 << 5;

pub const WPDMA_GLO_CFG: u32 = 0x0208;
pub const WPDMA_GLO_CFG_TX_DMA_BUSY: u32 = 1 << 1;
pub const WPDMA_GLO_CFG_RX_DMA_BUSY: u32 = 1 << 3;

/// Where the firmware goes: the FCE's DMA target and length, written with
/// the WRITE_FCE vendor request.
pub const FCE_DMA_ADDR: u16 = 0x0230;
pub const FCE_DMA_LEN: u16 = 0x0234;
pub const FCE_PSE_CTRL: u32 = 0x0800;
pub const TX_CPU_FROM_FCE_BASE_PTR: u32 = 0x09a0;
pub const TX_CPU_FROM_FCE_MAX_COUNT: u32 = 0x09a4;
pub const TX_CPU_FROM_FCE_CPU_DESC_IDX: u32 = 0x09a8;
pub const FCE_PDMA_GLOBAL_CONF: u32 = 0x09c4;
pub const FCE_SKIP_FS: u32 = 0x0a6c;

pub const MCU_CLOCK_CTL: u32 = 0x0708;
/// Bit 0: the firmware runs.
pub const MCU_COM_REG0: u32 = 0x0730;
/// The MAC's registers as the MCU's random read and write address them.
pub const MCU_MEMMAP_WLAN: u32 = 0x41_0000;

/// In the CFG space.
pub const USB_U3DMA_CFG: u32 = 0x9018;
pub const USB_DMA_CFG_RX_BULK_AGG_TOUT: u32 = 0xff;
pub const USB_DMA_CFG_RX_DROP_OR_PAD: u32 = 1 << 18;
pub const USB_DMA_CFG_RX_BULK_AGG_EN: u32 = 1 << 21;
pub const USB_DMA_CFG_RX_BULK_EN: u32 = 1 << 22;
pub const USB_DMA_CFG_TX_BULK_EN: u32 = 1 << 23;

/// In the CFG space (MT_WLAN_MTC_CTRL, as mt76x2u_power_on reaches it).
pub const WLAN_MTC_CTRL: u32 = 0x0148;
pub const WLAN_MTC_CTRL_MTCMOS_PWR_UP: u32 = 1 << 0;
pub const WLAN_MTC_CTRL_PWR_ACK: u32 = 1 << 12;
pub const WLAN_MTC_CTRL_PWR_ACK_S: u32 = 1 << 13;
pub const WLAN_MTC_CTRL_STATE_UP: u32 = 1 << 28;

/// From this revision (the low half of ASIC_VERSION) on, the ROM patch's
/// flag is in MCU_CLOCK_CTL and the DLM goes 0x800 further.
pub const REV_E3: u32 = 0x22;

// The MAC, as mt76x2u_mac_reset and mt76_write_mac_initvals set it up.
pub const PBF_SYS_CTRL: u32 = 0x0400;
pub const PBF_CFG: u32 = 0x0404;
pub const PBF_TX_MAX_PCNT: u32 = 0x0408;
pub const PBF_RX_MAX_PCNT: u32 = 0x040c;
pub const EFUSE_CTRL: u32 = 0x0024;
pub const COEXCFG0: u32 = 0x0040;
pub const COEXCFG0_COEX_EN: u32 = 1 << 0;
pub const XO_CTRL5: u32 = 0x0114;
pub const XO_CTRL5_C2_VAL: u32 = 0x7f << 8;
pub const XO_CTRL6: u32 = 0x0118;
pub const XO_CTRL6_C2_CTRL: u32 = 0x7f << 8;
pub const XO_CTRL7: u32 = 0x011c;
pub const WPDMA_DELAY_INT_CFG: u32 = 0x0210;
pub const WMM_AIFSN: u32 = 0x0214;
pub const WMM_CWMIN: u32 = 0x0218;
pub const WMM_CWMAX: u32 = 0x021c;
pub const TSO_CTRL: u32 = 0x0250;
pub const HEADER_TRANS_CTRL_REG: u32 = 0x0260;
pub const US_CYC_CFG: u32 = 0x02a4;
pub const US_CYC_CNT: u32 = 0xff;
pub const FCE_L2_STUFF: u32 = 0x080c;
pub const FCE_L2_STUFF_WR_MPDU_LEN_EN: u32 = 1 << 4;
pub const FCE_WLAN_FLOW_CONTROL1: u32 = 0x0824;
pub const PAUSE_ENABLE_CONTROL1: u32 = 0x0a38;
pub const MAC_SYS_CTRL: u32 = 0x1004;
pub const MAC_SYS_CTRL_RESET_CSR: u32 = 1 << 0;
pub const MAC_SYS_CTRL_RESET_BBP: u32 = 1 << 1;
pub const MAC_SYS_CTRL_ENABLE_TX: u32 = 1 << 2;
pub const MAC_SYS_CTRL_ENABLE_RX: u32 = 1 << 3;
pub const MAC_ADDR_DW0: u32 = 0x1008;
pub const MAC_ADDR_DW1: u32 = 0x100c;
pub const MAC_ADDR_DW1_U2ME_MASK: u32 = 0xff << 16;
pub const MAC_BSSID_DW0: u32 = 0x1010;
pub const MAC_BSSID_DW1: u32 = 0x1014;
pub const MAX_LEN_CFG: u32 = 0x1018;
pub const AMPDU_MAX_LEN_20M1S: u32 = 0x1030;
pub const AMPDU_MAX_LEN_20M2S: u32 = 0x1034;
pub const XIFS_TIME_CFG: u32 = 0x1100;
pub const XIFS_TIME_CFG_OFDM_SIFS: u32 = 0xff << 8;
pub const BKOFF_SLOT_CFG: u32 = 0x1104;
pub const BKOFF_SLOT_CFG_CC_DELAY: u32 = 0xf << 8;
pub const TBTT_SYNC_CFG: u32 = 0x1118;
pub const MAC_STATUS: u32 = 0x1200;
pub const PWR_PIN_CFG: u32 = 0x1204;
pub const AUX_CLK_CFG: u32 = 0x120c;
pub const DACCLK_EN_DLY_CFG: u32 = 0x1264;
pub const TX_PWR_CFG_0: u32 = 0x1314;
pub const TX_PWR_CFG_1: u32 = 0x1318;
pub const TX_PWR_CFG_2: u32 = 0x131c;
pub const TX_PWR_CFG_3: u32 = 0x1320;
pub const TX_PWR_CFG_4: u32 = 0x1324;
pub const TX_BAND_CFG: u32 = 0x132c;
pub const TX_BAND_CFG_UPPER_40M: u32 = 1 << 0;
pub const TX_BAND_CFG_5G: u32 = 1 << 1;
pub const TX_BAND_CFG_2G: u32 = 1 << 2;
pub const TX_SW_CFG0: u32 = 0x1330;
pub const TX_SW_CFG1: u32 = 0x1334;
pub const TX_SW_CFG2: u32 = 0x1338;
pub const TXOP_CTRL_CFG: u32 = 0x1340;
pub const TX_RTS_CFG: u32 = 0x1344;
pub const TX_TIMEOUT_CFG: u32 = 0x1348;
pub const TX_RETRY_CFG: u32 = 0x134c;
pub const TX_LINK_CFG: u32 = 0x1350;
pub const VHT_HT_FBK_CFG1: u32 = 0x1358;
pub const CCK_PROT_CFG: u32 = 0x1364;
pub const OFDM_PROT_CFG: u32 = 0x1368;
pub const MM20_PROT_CFG: u32 = 0x136c;
pub const MM40_PROT_CFG: u32 = 0x1370;
pub const GF20_PROT_CFG: u32 = 0x1374;
pub const GF40_PROT_CFG: u32 = 0x1378;
pub const EXP_ACK_TIME: u32 = 0x1380;
pub const HT_FBK_TO_LEGACY: u32 = 0x1384;
pub const TX_ALC_CFG_4: u32 = 0x13c0;
pub const TX_ALC_VGA3: u32 = 0x13c8;
pub const TX_PWR_CFG_7: u32 = 0x13d4;
pub const TX_PWR_CFG_8: u32 = 0x13d8;
pub const TX_PWR_CFG_9: u32 = 0x13dc;
pub const TX_PROT_CFG6: u32 = 0x13e0;
pub const TX_PROT_CFG7: u32 = 0x13e4;
pub const TX_PROT_CFG8: u32 = 0x13e8;
pub const PIFS_TX_CFG: u32 = 0x13ec;
pub const RX_FILTR_CFG: u32 = 0x1400;
pub const AUTO_RSP_CFG: u32 = 0x1404;
pub const LEGACY_BASIC_RATE: u32 = 0x1408;
pub const HT_BASIC_RATE: u32 = 0x140c;
pub const HT_CTRL_CFG: u32 = 0x1410;
pub const EXT_CCA_CFG: u32 = 0x141c;
pub const TX_SW_CFG3: u32 = 0x1478;
pub const PN_PAD_MODE: u32 = 0x150c;
pub const TXOP_HLDR_ET: u32 = 0x1608;
pub const PROT_AUTO_TX_CFG: u32 = 0x1648;

/// RX_FILTR_CFG: each bit drops what it names.
pub mod rx_filter {
    pub const CRC_ERR: u32 = 1 << 0;
    pub const PHY_ERR: u32 = 1 << 1;
    pub const PROMISC: u32 = 1 << 2;
    pub const VER_ERR: u32 = 1 << 4;
    pub const DUP: u32 = 1 << 7;
    pub const CFACK: u32 = 1 << 8;
    pub const CFEND: u32 = 1 << 9;
    pub const ACK: u32 = 1 << 10;
    pub const CTS: u32 = 1 << 11;
    pub const RTS: u32 = 1 << 12;
    pub const PSPOLL: u32 = 1 << 13;
    pub const CTRL_RSV: u32 = 1 << 16;
}

/// The baseband's registers (MT_BBP(type, n)).
pub const fn bbp(base: u32, n: u32) -> u32 {
    base + (n << 2)
}
pub const BBP_CORE: u32 = 0x2000;
pub const BBP_IBI: u32 = 0x2100;
pub const BBP_AGC: u32 = 0x2300;
pub const BBP_TXO: u32 = 0x2600;
pub const BBP_TXBE: u32 = 0x2700;
pub const BBP_RXO: u32 = 0x2900;
pub const BBP_CORE_R1_BW: u32 = 0x3 << 3;
pub const BBP_AGC_R0_CTRL_CHAN: u32 = 0x3 << 8;
pub const BBP_AGC_R0_BW: u32 = 0x7 << 12;
pub const BBP_TXBE_R0_CTRL_CHAN: u32 = 0x3;

/// EXT_CCA_CFG's fields.
pub const EXT_CCA_CFG_CCA_FIELDS: u32 = 0xfff;

/// EEPROM offsets (mt76x02_eeprom.h), each a 16-bit value.
pub mod ee {
    pub const NIC_CONF_0: usize = 0x034;
    /// The 5 GHz power amplifier is the chip's own (no external PA).
    pub const NIC_CONF_0_PA_INT_5G: u16 = 1 << 9;
    /// Each chain's 5 GHz power: 5 bytes per calibration group.
    pub const TX_POWER_0_START_5G: usize = 0x062;
    pub const TX_POWER_1_START_5G: usize = 0x080;
    pub const NIC_CONF_1: usize = 0x036;
    pub const NIC_CONF_1_TEMP_TX_ALC: u16 = 1 << 1;
    pub const NIC_CONF_1_TX_ALC_EN: u16 = 1 << 13;
    pub const TX_POWER_DELTA_BW40: usize = 0x050;
    pub const TX_POWER_EXT_PA_5G: usize = 0x054;
    /// Per-rate power, a byte for each pair of rates (mt76x2_get_rate_power).
    pub const TX_POWER_CCK: usize = 0x0a0;
    pub const TX_POWER_HT_MCS0: usize = 0x0a6;
    pub const TX_POWER_HT_MCS4: usize = 0x0a8;
    pub const TX_POWER_HT_MCS8: usize = 0x0aa;
    pub const TX_POWER_HT_MCS12: usize = 0x0ac;
    pub const TX_POWER_OFDM_5G_6M: usize = 0x0b2;
    pub const TX_POWER_OFDM_5G_24M: usize = 0x0b4;
    pub const TX_POWER_VHT_MCS8: usize = 0x0be;
    /// Its low byte is the target power (mt76x2_get_power_info_5g).
    pub const RF_2G_RX_HIGH_GAIN: usize = 0x0f8;
    pub const XTAL_TRIM_1: usize = 0x03a;
    pub const NIC_CONF_2: usize = 0x042;
    pub const NIC_CONF_2_XTAL_OPTION: u16 = 0x3 << 9;
    pub const XTAL_TRIM_2: usize = 0x09e;
    pub const BT_RCAL_RESULT: usize = 0x138;
}

// Transmit power (mt76x2_phy_set_txpower_regs, mt76x02_phy_set_txpower).
pub const BB_PA_MODE_CFG0: u32 = 0x1214;
pub const BB_PA_MODE_CFG1: u32 = 0x1218;
pub const RF_PA_MODE_CFG0: u32 = 0x121c;
pub const RF_PA_MODE_CFG1: u32 = 0x1220;
pub const RF_PA_MODE_ADJ0: u32 = 0x1228;
pub const RF_PA_MODE_ADJ1: u32 = 0x122c;
pub const TX0_RF_GAIN_CORR: u32 = 0x13a0;
pub const TX1_RF_GAIN_CORR: u32 = 0x13a4;
pub const TX_ALC_CFG_2: u32 = 0x13a8;
pub const TX_ALC_CFG_3: u32 = 0x13ac;
pub const TX_ALC_CFG_0: u32 = 0x13b0;
pub const TX_ALC_CFG_0_CH_INIT_0: u32 = 0x3f;
pub const TX_ALC_CFG_0_CH_INIT_1: u32 = 0x3f << 8;
pub const TX_ALC_CFG_1: u32 = 0x13b4;
/// TX_ALC_CFG_1's and TX_ALC_CFG_2's temperature compensation.
pub const TX_ALC_CFG_TEMP_COMP: u32 = 0x3f;

/// The WCID table (MT_WCID_ADDR, MT_WCID_ATTR): a station's address, and
/// its receive Block Ack bits, one per TID, in the 16 bits after it.
pub const fn wcid_addr(n: u32) -> u32 {
    0x1800 + n * 8
}
pub const fn wcid_attr(n: u32) -> u32 {
    0xa800 + n * 4
}

/// The two words of a WCID address entry (struct mt76_wcid_addr): the MAC
/// address, then the Block Ack mask.
pub fn wcid_words(mac: [u8; 6], ba_mask: u16) -> [u32; 2] {
    [
        u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]),
        u32::from(u16::from_le_bytes([mac[4], mac[5]])) | (u32::from(ba_mask) << 16),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wcid_entry_holds_the_address_then_the_block_ack_bits() {
        let mac = [0x4e, 0x4f, 0x7b, 0xa7, 0xf2, 0x70];
        assert_eq!(wcid_words(mac, 1 << 5), [0xa77b_4f4e, 0x0020_70f2]);
        assert_eq!(wcid_addr(1), 0x1808);
        assert_eq!(wcid_attr(2), 0xa808);
    }
}
