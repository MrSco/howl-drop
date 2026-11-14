//! Frames to send, as one bulk OUT transfer each (mt76x02u_tx_prepare_skb,
//! mt76x02_mac_write_txwi): the DMA info word, the TXWI descriptor (20
//! bytes), the 802.11 frame (2 pad bytes after a header whose length isn't a
//! multiple of 4), zeros to a multiple of 4, then 4 more zeros.

use crate::rx::header_len;

/// A transmit rate, as MT_RXWI_RATE_* encodes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    /// A non-HT rate in units of 500 kbit/s (radiotap's): 2, 4, 11, 22 are
    /// CCK, 12 to 108 OFDM.
    Legacy(u8),
    /// An HT MCS on 20 MHz.
    Ht { mcs: u8, sgi: bool },
    /// An HT MCS across the 40 MHz the stick is tuned to (its primary
    /// channel and the one above).
    Ht40 { mcs: u8, sgi: bool },
}

const PHY_CCK: u16 = 0;
const PHY_OFDM: u16 = 1;
const PHY_HT: u16 = 2;

impl Rate {
    /// The TXWI's rate field, or None for a rate the chip doesn't have.
    pub fn value(self) -> Option<u16> {
        let (phy, index, sgi) = match self {
            Rate::Legacy(r) => {
                let cck = [2, 4, 11, 22].iter().position(|&c| c == r);
                let ofdm = [12, 18, 24, 36, 48, 72, 96, 108]
                    .iter()
                    .position(|&o| o == r);
                match (cck, ofdm) {
                    (Some(i), _) => (PHY_CCK, i as u16, false),
                    (_, Some(i)) => (PHY_OFDM, i as u16, false),
                    _ => return None,
                }
            }
            Rate::Ht { mcs, sgi } if mcs <= 15 => (PHY_HT, u16::from(mcs), sgi),
            Rate::Ht40 { mcs, sgi } if mcs <= 15 => {
                return Some(u16::from(mcs) | BW_40 | (u16::from(sgi) << 9) | (PHY_HT << 13));
            }
            Rate::Ht { .. } | Rate::Ht40 { .. } => return None,
        };
        Some(index | (u16::from(sgi) << 9) | (phy << 13))
    }

    /// The rate in units of 100 kbit/s: what a frame's time on air goes by.
    pub fn kbps100(self) -> u32 {
        // HT MCS 0-7 on one stream, long GI: 20 MHz, then 40 MHz.
        const HT20: [u32; 8] = [65, 130, 195, 260, 390, 520, 585, 650];
        const HT40: [u32; 8] = [135, 270, 405, 540, 810, 1080, 1215, 1350];
        let ht = |table: &[u32; 8], mcs: u8, sgi: bool| {
            let streams = u32::from(mcs / 8) + 1;
            let r = table[usize::from(mcs % 8)] * streams;
            if sgi { r * 10 / 9 } else { r }
        };
        match self {
            Rate::Legacy(r) => u32::from(r) * 5,
            Rate::Ht { mcs, sgi } => ht(&HT20, mcs.min(15), sgi),
            Rate::Ht40 { mcs, sgi } => ht(&HT40, mcs.min(15), sgi),
        }
    }
}

/// MT_RXWI_RATE_BW: 40 MHz.
const BW_40: u16 = 1 << 7;

/// The DMA port and queue frames go to (WLAN_PORT, MT_QSEL_EDCA).
const WLAN_PORT: u32 = 0;
const QSEL_EDCA: u32 = 2;
/// MT_TXD_INFO_80211 and MT_TXD_INFO_WIV.
const INFO_80211: u32 = 1 << 19;
const INFO_WIV: u32 = 1 << 24;
/// MT_TXWI_ACK_CTL_REQ.
const ACK_REQ: u8 = 1 << 0;
/// No station entry (WCID) for the frame.
const NO_WCID: u8 = 0xff;
/// Two streams on revision E4 and later (mt76x02_mac_write_txwi).
const TXSTREAM_2: u8 = 0x13;

const TXWI_LEN: usize = 20;

/// The bulk OUT transfer for `frame` (from its MAC header, without FCS). A
/// frame to a group address never asks for an ACK, whatever `no_ack` says,
/// as mac80211 sends it: nobody answers one, and the chip would retry it
/// until its retry limit, holding up every frame behind it.
pub fn packet(frame: &[u8], rate: u16, no_ack: bool) -> Vec<u8> {
    packet_with(frame, rate, no_ack, None)
}

/// A frame the chip may aggregate into an A-MPDU: the peer's WCID entry,
/// and the Block Ack window it agreed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ampdu {
    pub wcid: u8,
    pub window: u16,
}

/// MT_TXWI_FLAGS_AMPDU, and MT_TXWI_FLAGS_MPDU_DENSITY at 4 us
/// (IEEE80211_HT_MPDU_DENSITY_4, the least mt76x02_mac_write_txwi uses).
const FLAGS_AMPDU: u16 = 1 << 4;
const MPDU_DENSITY_4US: u16 = 5 << 5;

/// As [`packet`], marked for aggregation when `ampdu` is given
/// (mt76x02_mac_write_txwi for IEEE80211_TX_CTL_AMPDU).
pub fn packet_with(frame: &[u8], rate: u16, no_ack: bool, ampdu: Option<Ampdu>) -> Vec<u8> {
    let hdr = header_len(frame).unwrap_or(frame.len()).min(frame.len());
    let pad = if hdr.is_multiple_of(4) { 0 } else { 2 };
    let group = frame.get(4).is_some_and(|b| b & 1 != 0);

    let mut txwi = [0u8; TXWI_LEN];
    txwi[2..4].copy_from_slice(&rate.to_le_bytes());
    txwi[4] = if no_ack || group { 0 } else { ACK_REQ };
    txwi[5] = NO_WCID;
    if let Some(a) = ampdu {
        let window = (a.window.clamp(1, 64) - 1) as u8;
        txwi[0..2].copy_from_slice(&(FLAGS_AMPDU | MPDU_DENSITY_4US).to_le_bytes());
        txwi[4] |= window << 2;
        txwi[5] = a.wcid;
    }
    txwi[6..8].copy_from_slice(&(frame.len() as u16).to_le_bytes());
    txwi[17] = TXSTREAM_2;

    let body_len = TXWI_LEN + frame.len() + pad;
    let info = (body_len.div_ceil(4) * 4) as u32
        | (WLAN_PORT << 27)
        | (QSEL_EDCA << 25)
        | INFO_80211
        | INFO_WIV;
    let mut out = Vec::with_capacity(body_len + 12);
    out.extend_from_slice(&info.to_le_bytes());
    out.extend_from_slice(&txwi);
    out.extend_from_slice(&frame[..hdr]);
    out.resize(out.len() + pad, 0);
    out.extend_from_slice(&frame[hdr..]);
    out.resize(4 + body_len.div_ceil(4) * 4 + 4, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_encode_as_mt76_does() {
        // 12 Mbit/s: OFDM index 2, as the iPhone's action frames came in
        assert_eq!(Rate::Legacy(24).value(), Some(0x2002));
        assert_eq!(Rate::Legacy(72).value(), Some(0x2005));
        assert_eq!(Rate::Legacy(2).value(), Some(0x0000));
        assert_eq!(Rate::Ht { mcs: 7, sgi: true }.value(), Some(0x4207));
        assert_eq!(Rate::Ht40 { mcs: 7, sgi: false }.value(), Some(0x4087));
        // An A-MPDU's frame: flags, Block Ack window less one, the WCID.
        let f = [0x88u8; 26];
        let p = packet_with(
            &f,
            0x2007,
            false,
            Some(Ampdu {
                wcid: 3,
                window: 64,
            }),
        );
        let txwi = &p[4..24];
        assert_eq!(u16::from_le_bytes([txwi[0], txwi[1]]), (1 << 4) | (5 << 5));
        assert_eq!(txwi[4], 1 | (63 << 2));
        assert_eq!(txwi[5], 3);
        assert_eq!(
            Rate::Ht40 {
                mcs: 15,
                sgi: false
            }
            .kbps100(),
            2700
        );
        assert_eq!(Rate::Legacy(108).kbps100(), 540);
        assert_eq!(Rate::Ht { mcs: 7, sgi: true }.kbps100(), 722);
        assert_eq!(Rate::Legacy(13).value(), None);
    }

    #[test]
    fn an_action_frame_goes_unpadded() {
        let frame = [0xd0u8; 30];
        let p = packet(&frame, 0x2002, true);
        let info = u32::from_le_bytes(p[..4].try_into().unwrap());
        assert_eq!(info & 0xffff, 52); // 20 + 30, to a multiple of 4
        assert_eq!((info >> 25) & 0x3, QSEL_EDCA);
        assert_ne!(info & INFO_80211, 0);
        assert_eq!(&p[6..8], &0x2002u16.to_le_bytes());
        assert_eq!(p[8], 0, "no ACK asked");
        assert_eq!(p[9], 0xff);
        assert_eq!(&p[10..12], &30u16.to_le_bytes());
        assert_eq!(&p[24..54], &frame);
        assert_eq!(p.len(), 4 + 52 + 4);
    }

    #[test]
    fn broadcast_never_asks_for_an_ack() {
        let mut frame = vec![0xd0, 0x00, 0x00, 0x00];
        frame.extend_from_slice(&[0xff; 6]);
        frame.extend_from_slice(&[0x02; 20]);
        assert_eq!(packet(&frame, 0x2002, false)[8], 0);
        frame[4..10].copy_from_slice(&[0x4e, 0x4f, 0x7b, 0xa7, 0xf2, 0x70]);
        assert_eq!(packet(&frame, 0x2002, false)[8], ACK_REQ);
    }

    #[test]
    fn a_qos_data_header_gets_two_pad_bytes() {
        let mut frame = vec![0x88, 0x00];
        frame.extend_from_slice(&[0x22; 24]); // to a 26-byte QoS header, unicast
        frame.extend_from_slice(&[0xaa; 8]);
        let p = packet(&frame, 0x2005, false);
        assert_eq!(p[8], ACK_REQ);
        assert_eq!(&p[24..50], &frame[..26]);
        assert_eq!(&p[50..52], &[0, 0]);
        assert_eq!(&p[52..60], &[0xaa; 8]);
    }
}
