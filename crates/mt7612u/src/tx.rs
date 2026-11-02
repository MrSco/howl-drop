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
            Rate::Ht { .. } => return None,
        };
        Some(index | (u16::from(sgi) << 9) | (phy << 13))
    }
}

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
    let hdr = header_len(frame).unwrap_or(frame.len()).min(frame.len());
    let pad = if hdr.is_multiple_of(4) { 0 } else { 2 };
    let group = frame.get(4).is_some_and(|b| b & 1 != 0);

    let mut txwi = [0u8; TXWI_LEN];
    txwi[2..4].copy_from_slice(&rate.to_le_bytes());
    txwi[4] = if no_ack || group { 0 } else { ACK_REQ };
    txwi[5] = NO_WCID;
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
