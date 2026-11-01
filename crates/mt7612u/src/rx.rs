//! Received frames, as the bulk IN endpoint delivers them (mt76u_rx,
//! mt76x02_mac_process_rx): a 4-byte length, the RXWI descriptor (32 bytes),
//! the 802.11 frame (2 pad bytes after its header when RXINFO_L2PAD says so),
//! then 4 bytes of FCE info.

const DMA_HDR_LEN: usize = 4;
const RXWI_LEN: usize = 32;
const FCE_INFO_LEN: usize = 4;

/// RXWI's rxinfo bits (MT_RXINFO_*).
pub mod info {
    pub const CRCERR: u32 = 1 << 8;
    pub const L2PAD: u32 = 1 << 14;
    pub const AMPDU: u32 = 1 << 15;
    pub const PN_LEN: u32 = 0x7 << 19;
}

/// One frame and what the chip says about it.
#[derive(Debug, PartialEq, Eq)]
pub struct RxFrame {
    /// The 802.11 frame, header to body, without FCS or padding.
    pub frame: Vec<u8>,
    pub rxinfo: u32,
    /// MT_RXWI_RATE_*: index, bandwidth, PHY type.
    pub rate: u16,
    /// Per chain, as the chip reports it (not yet calibrated to dBm).
    pub rssi: [u8; 2],
}

impl RxFrame {
    pub fn crc_error(&self) -> bool {
        self.rxinfo & info::CRCERR != 0
    }
}

/// The frame in one bulk IN transfer, or None for one that doesn't hold a
/// whole frame (mt76u_get_rx_entry_len's checks).
pub fn parse(data: &[u8]) -> Option<RxFrame> {
    if data.len() < DMA_HDR_LEN + RXWI_LEN + FCE_INFO_LEN {
        return None;
    }
    let dma_len = u16::from_le_bytes([data[0], data[1]]) as usize;
    if dma_len == 0 || !dma_len.is_multiple_of(4) || DMA_HDR_LEN + dma_len > data.len() {
        return None;
    }
    let le32 = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    let rxwi = DMA_HDR_LEN;
    let rxinfo = le32(rxwi);
    let ctl = le32(rxwi + 4);
    let rate = u16::from_le_bytes([data[rxwi + 10], data[rxwi + 11]]);
    let rssi = [data[rxwi + 12], data[rxwi + 13]];
    let mpdu_len = ((ctl >> 16) & 0x3fff) as usize;

    let body = &data[rxwi + RXWI_LEN..DMA_HDR_LEN + dma_len];
    let pad = if rxinfo & info::L2PAD != 0 { 2 } else { 0 };
    let hdr_len = header_len(body)?.min(mpdu_len);
    if hdr_len + pad + (mpdu_len - hdr_len) > body.len() {
        return None;
    }
    let mut frame = body[..hdr_len].to_vec();
    frame.extend_from_slice(&body[hdr_len + pad..pad + mpdu_len]);
    Some(RxFrame {
        frame,
        rxinfo,
        rate,
        rssi,
    })
}

/// An 802.11 header's length (ieee80211_hdrlen), from its frame control.
pub fn header_len(frame: &[u8]) -> Option<usize> {
    let fc = u16::from_le_bytes([*frame.first()?, *frame.get(1)?]);
    let (ftype, subtype) = ((fc >> 2) & 0x3, (fc >> 4) & 0xf);
    Some(match ftype {
        // management
        0 => 24,
        // control: ACK and CTS have one address, the rest two
        1 => match subtype {
            0xc | 0xd => 10,
            _ => 16,
        },
        // data: a fourth address with ToDS and FromDS, QoS 2 more
        2 => {
            let mut len = 24;
            if fc & 0x0300 == 0x0300 {
                len += 6;
            }
            if subtype & 0x8 != 0 {
                len += 2;
            }
            len
        }
        _ => return None,
    })
}

/// Whether a frame is an AWDL action frame: a management action frame,
/// category 127 (vendor), Apple's OUI 00:17:f2, type 8.
pub fn is_awdl(frame: &[u8]) -> bool {
    frame.len() >= 24 + 5
        && frame[0] == 0xd0
        && frame[24] == 127
        && frame[25..28] == [0x00, 0x17, 0xf2]
        && frame[28] == 0x08
}

/// A frame's transmitter (addr2), when it has one.
pub fn transmitter(frame: &[u8]) -> Option<[u8; 6]> {
    if header_len(frame)? < 16 {
        return None;
    }
    frame.get(10..16)?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bulk IN transfer holding `frame`, padded after its 24-byte header if
    /// `pad`.
    fn entry(frame: &[u8], pad: bool) -> Vec<u8> {
        let mut rxwi = vec![0u8; RXWI_LEN];
        let rxinfo = if pad { info::L2PAD } else { 0 };
        rxwi[..4].copy_from_slice(&rxinfo.to_le_bytes());
        rxwi[4..8].copy_from_slice(&((frame.len() as u32) << 16).to_le_bytes());
        rxwi[12] = 0xd5;
        let mut body = frame[..24].to_vec();
        if pad {
            body.extend_from_slice(&[0xee, 0xee]);
        }
        body.extend_from_slice(&frame[24..]);
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
        let dma_len = RXWI_LEN + body.len();
        let mut out = (dma_len as u32).to_le_bytes().to_vec();
        out.extend_from_slice(&rxwi);
        out.extend_from_slice(&body);
        out.extend_from_slice(&[0; FCE_INFO_LEN]);
        out
    }

    fn awdl_action() -> Vec<u8> {
        let mut f = vec![0xd0, 0x00, 0, 0];
        f.extend_from_slice(&[0xff; 6]); // addr1: broadcast
        f.extend_from_slice(&[0x4e, 0x4f, 0x7b, 0xa7, 0xf2, 0x70]); // addr2
        f.extend_from_slice(&[0x00, 0x25, 0x00, 0xff, 0x94, 0x73]); // AWDL BSSID
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(&[127, 0x00, 0x17, 0xf2, 0x08, 0x10, 0x00]);
        f
    }

    #[test]
    fn an_awdl_frame_comes_out_whole() {
        for pad in [false, true] {
            let rx = parse(&entry(&awdl_action(), pad)).unwrap();
            assert_eq!(rx.frame, awdl_action(), "pad {pad}");
            assert!(is_awdl(&rx.frame));
            assert_eq!(
                transmitter(&rx.frame),
                Some([0x4e, 0x4f, 0x7b, 0xa7, 0xf2, 0x70])
            );
            assert_eq!(rx.rssi[0], 0xd5);
        }
    }

    #[test]
    fn short_or_odd_transfers_are_left() {
        assert_eq!(parse(&[0; 8]), None);
        let mut bad = entry(&awdl_action(), false);
        bad[0] = 3; // not a multiple of 4
        assert_eq!(parse(&bad), None);
    }

    #[test]
    fn header_lengths() {
        assert_eq!(header_len(&[0x80, 0]), Some(24)); // beacon
        assert_eq!(header_len(&[0xd4, 0]), Some(10)); // ACK
        assert_eq!(header_len(&[0x88, 0x01]), Some(26)); // QoS data
        assert_eq!(header_len(&[0x88, 0x03]), Some(32)); // QoS data, 4 addresses
        assert!(!is_awdl(&[0x80; 40]));
    }
}
