//! The MCU's framing over bulk USB, as mt76x02_usb_mcu.c and mt76x2/usb_mcu.c
//! build it: the firmware files' headers, firmware chunks, commands and their
//! responses. Pure functions; [`crate::Mt7612u`] sends what they build.

use std::fmt;

/// Where the ROM patch, the firmware's ILM and its DLM go in the MCU's memory.
pub const ROM_PATCH_OFFSET: u32 = 0x9_0000;
pub const ILM_OFFSET: u32 = 0x8_0000;
pub const DLM_OFFSET: u32 = 0x11_0000;
/// The largest bulk transfer of each, header included.
pub const ROM_PATCH_MAX_PAYLOAD: usize = 2048;
pub const FW_MAX_PAYLOAD: usize = 0x3900;
/// A response's bulk IN transfer (MCU_RESP_URB_SIZE).
pub const RESP_SIZE: usize = 1024;
/// The most bytes one command carries (MT_INBAND_PACKET_MAX_LEN).
pub const INBAND_MAX: usize = 192;

/// The DMA port commands and firmware go to (CPU_TX_PORT).
const CPU_TX_PORT: u32 = 2;
/// MT_MCU_MSG_TYPE_CMD.
const MSG_TYPE_CMD: u32 = 1 << 30;

/// Commands (mt76x02_mcu.h, enum mcu_cmd).
pub mod cmd {
    pub const FUN_SET_OP: u8 = 1;
    pub const LOAD_CR: u8 = 2;
    pub const INIT_GAIN_OP: u8 = 3;
    pub const RANDOM_READ: u8 = 10;
    pub const POWER_SAVING_OP: u8 = 20;
    pub const SWITCH_CHANNEL_OP: u8 = 30;
    pub const CALIBRATION_OP: u8 = 31;
}

/// CALIBRATION_OP's calibrations (mt76x2/mcu.h, enum mcu_calibration).
pub mod cal {
    pub const R: u32 = 1;
    pub const TEMP_SENSOR: u32 = 2;
    pub const RXDCOC: u32 = 3;
    pub const RC: u32 = 4;
    pub const LC: u32 = 6;
    pub const TX_LOFT: u32 = 7;
    pub const TXIQ: u32 = 8;
    pub const RXIQC_FI: u32 = 12;
    pub const TX_SHAPING: u32 = 15;
}

/// LOAD_CR's mode: the RF's and the baseband's registers (MT_RF_BBP_CR).
pub const RF_BBP_CR: u8 = 2;

/// Two little-endian words: most commands' payload.
pub fn words(a: u32, b: u32) -> Vec<u8> {
    let mut out = a.to_le_bytes().to_vec();
    out.extend_from_slice(&b.to_le_bytes());
    out
}

/// FUN_SET_OP's function: the queue the MCU answers on (Q_SELECT).
pub const Q_SELECT: u32 = 1;
/// POWER_SAVING_OP's mode.
pub const RADIO_ON: u32 = 0x31;

#[derive(Debug, PartialEq, Eq)]
pub enum FirmwareError {
    TooShort,
    /// The header's ILM and DLM lengths don't add up to the file.
    BadLength {
        header: usize,
        file: usize,
    },
}

impl fmt::Display for FirmwareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FirmwareError::TooShort => write!(f, "the file is shorter than its header"),
            FirmwareError::BadLength { header, file } => {
                write!(f, "the header says {header} bytes, the file has {file}")
            }
        }
    }
}

/// mt7662_rom_patch.bin: a 30-byte header (mt76x02_patch_header), then the
/// patch.
pub struct RomPatch<'a> {
    pub build_time: String,
    pub body: &'a [u8],
}

const PATCH_HEADER_LEN: usize = 16 + 4 + 4 + 4 + 2;

impl<'a> RomPatch<'a> {
    pub fn parse(file: &'a [u8]) -> Result<Self, FirmwareError> {
        if file.len() <= PATCH_HEADER_LEN {
            return Err(FirmwareError::TooShort);
        }
        Ok(RomPatch {
            build_time: text(&file[..15]),
            body: &file[PATCH_HEADER_LEN..],
        })
    }
}

/// mt7662.bin: a 32-byte header (mt76x02_fw_header), then the ILM and DLM.
pub struct Firmware<'a> {
    pub build_ver: u16,
    pub fw_ver: u16,
    pub build_time: String,
    pub ilm: &'a [u8],
    pub dlm: &'a [u8],
}

const FW_HEADER_LEN: usize = 4 + 4 + 2 + 2 + 4 + 16;

impl<'a> Firmware<'a> {
    pub fn parse(file: &'a [u8]) -> Result<Self, FirmwareError> {
        if file.len() < FW_HEADER_LEN {
            return Err(FirmwareError::TooShort);
        }
        let le32 = |at: usize| u32::from_le_bytes(file[at..at + 4].try_into().unwrap()) as usize;
        let le16 = |at: usize| u16::from_le_bytes(file[at..at + 2].try_into().unwrap());
        let (ilm_len, dlm_len) = (le32(0), le32(4));
        let header = FW_HEADER_LEN + ilm_len + dlm_len;
        if header != file.len() {
            return Err(FirmwareError::BadLength {
                header,
                file: file.len(),
            });
        }
        let ilm = &file[FW_HEADER_LEN..FW_HEADER_LEN + ilm_len];
        Ok(Firmware {
            build_ver: le16(8),
            fw_ver: le16(10),
            build_time: text(&file[16..32]),
            ilm,
            dlm: &file[FW_HEADER_LEN + ilm_len..],
        })
    }

    /// "0.0.00", as the Linux driver prints it.
    pub fn version(&self) -> String {
        let v = self.fw_ver;
        format!("{}.{}.{:02}", (v >> 12) & 0xf, (v >> 8) & 0xf, v & 0xf)
    }
}

fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// A piece of firmware as one bulk transfer (__mt76x02u_mcu_fw_send_data):
/// its info word, the bytes, zeros to a multiple of 4, then 4 more zeros.
pub fn firmware_chunk(data: &[u8]) -> Vec<u8> {
    let info = (CPU_TX_PORT << 27) | data.len() as u32 | MSG_TYPE_CMD;
    let mut out = Vec::with_capacity(data.len() + 12);
    out.extend_from_slice(&info.to_le_bytes());
    out.extend_from_slice(data);
    out.resize(4 + round4(data.len()) + 4, 0);
    out
}

/// The FCE_DMA_LEN value for a chunk of `len` bytes.
pub fn chunk_dma_len(len: usize) -> u32 {
    (round4(len) as u32) << 16
}

/// A command as one bulk transfer (mt76x02u_skb_dma_info): its info word
/// (length, port, sequence, command), the payload, zeros to a multiple of 4,
/// then 4 more zeros. `seq` 0 asks for no response.
pub fn command(cmd: u8, seq: u8, payload: &[u8]) -> Vec<u8> {
    let info = round4(payload.len()) as u32
        | (CPU_TX_PORT << 27)
        | (u32::from(seq & 0xf) << 16)
        | (u32::from(cmd & 0x7f) << 20)
        | MSG_TYPE_CMD;
    let mut out = Vec::with_capacity(payload.len() + 12);
    out.extend_from_slice(&info.to_le_bytes());
    out.extend_from_slice(payload);
    out.resize(4 + round4(payload.len()) + 4, 0);
    out
}

/// A response's info word (MT_RX_FCE_INFO): its sequence and event type.
#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub seq: u8,
    /// 0 is EVT_CMD_DONE.
    pub event: u8,
}

pub fn response(data: &[u8]) -> Option<Response> {
    let info = u32::from_le_bytes(data.get(..4)?.try_into().ok()?);
    Some(Response {
        seq: ((info >> 16) & 0xf) as u8,
        event: ((info >> 20) & 0xf) as u8,
    })
}

/// RANDOM_READ's payload: each register's address (base added) and a zero.
pub fn random_read(base: u32, regs: &[u32]) -> Vec<u8> {
    regs.iter()
        .flat_map(|&r| {
            let mut pair = (base + r).to_le_bytes().to_vec();
            pair.extend_from_slice(&0u32.to_le_bytes());
            pair
        })
        .collect()
}

/// RANDOM_READ's answer: after the info word, an (address, value) pair per
/// register, base added to the address.
pub fn random_read_values(data: &[u8], base: u32, regs: &[u32]) -> Option<Vec<u32>> {
    let le32 = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
    };
    regs.iter()
        .enumerate()
        .map(|(i, &r)| {
            let at = 4 + 8 * i;
            (le32(at)? == base + r).then_some(())?;
            le32(at + 4)
        })
        .collect()
}

fn round4(n: usize) -> usize {
    n.div_ceil(4) * 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_firmware_chunk_is_framed_as_mt76_sends_it() {
        let chunk = firmware_chunk(&[1, 2, 3, 4, 5]);
        // len 5, port 2, a command
        assert_eq!(&chunk[..4], &(0x4000_0000u32 | (2 << 27) | 5).to_le_bytes());
        assert_eq!(&chunk[4..9], &[1, 2, 3, 4, 5]);
        assert_eq!(chunk.len(), 4 + 8 + 4);
        assert!(chunk[9..].iter().all(|&b| b == 0));
        assert_eq!(chunk_dma_len(5), 8 << 16);
    }

    #[test]
    fn a_command_carries_its_sequence_and_number() {
        let msg = command(cmd::RANDOM_READ, 3, &[0xaa; 8]);
        let info = u32::from_le_bytes(msg[..4].try_into().unwrap());
        assert_eq!(info & 0xffff, 8);
        assert_eq!((info >> 16) & 0xf, 3);
        assert_eq!((info >> 20) & 0x7f, 10);
        assert_eq!((info >> 27) & 0x7, 2);
        assert_eq!(info >> 30, 1);
        assert_eq!(msg.len(), 4 + 8 + 4);
    }

    #[test]
    fn a_response_reads_back() {
        // Sequence 5, event 0 (done), 12 bytes.
        let info: u32 = (5 << 16) | 12;
        let mut data = info.to_le_bytes().to_vec();
        data.extend_from_slice(&(MCU_MEMMAP).to_le_bytes());
        data.extend_from_slice(&0x7612_0044u32.to_le_bytes());
        assert_eq!(response(&data), Some(Response { seq: 5, event: 0 }));
        assert_eq!(
            random_read_values(&data, MCU_MEMMAP, &[0]),
            Some(vec![0x7612_0044])
        );
        // Another register's answer isn't taken for this one.
        assert_eq!(random_read_values(&data, MCU_MEMMAP, &[4]), None);
    }

    const MCU_MEMMAP: u32 = 0x41_0000;

    #[test]
    fn a_random_read_asks_for_each_register() {
        let payload = random_read(MCU_MEMMAP, &[0, 0x1000]);
        assert_eq!(payload.len(), 16);
        assert_eq!(&payload[..4], &0x41_0000u32.to_le_bytes());
        assert_eq!(&payload[8..12], &0x41_1000u32.to_le_bytes());
    }

    #[test]
    fn firmware_headers_parse() {
        let mut fw = vec![0u8; 32];
        fw[0..4].copy_from_slice(&3u32.to_le_bytes());
        fw[4..8].copy_from_slice(&2u32.to_le_bytes());
        fw[8..10].copy_from_slice(&1u16.to_le_bytes());
        fw[16..32].copy_from_slice(b"201507311614____");
        fw.extend_from_slice(&[9, 9, 9, 7, 7]);
        let f = Firmware::parse(&fw).unwrap();
        assert_eq!((f.ilm, f.dlm), (&[9, 9, 9][..], &[7, 7][..]));
        assert_eq!(f.version(), "0.0.00");
        assert_eq!(f.build_time, "201507311614____");
        fw.push(0);
        assert!(matches!(
            Firmware::parse(&fw),
            Err(FirmwareError::BadLength { .. })
        ));

        let mut patch = b"20141115060606a\0".to_vec();
        patch.resize(30, 0);
        patch.extend_from_slice(&[1, 2]);
        let p = RomPatch::parse(&patch).unwrap();
        assert_eq!(p.build_time, "20141115060606a");
        assert_eq!(p.body, &[1, 2]);
    }
}
