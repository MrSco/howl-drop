//! A user-space driver for the MediaTek MT7612U (Alfa AWUS036ACM, Netgear
//! A6210 and others): enough of Linux's mt76x2u (openwrt/mt76) to run the chip
//! as a monitor-and-inject radio for owly-awdl, over nusb: WinUSB on Windows,
//! usbfs on Linux. docs/native-windows.md has the plan.
//!
//! So far: finding a stick, and reading its registers and EEPROM.

use std::fmt;
use std::thread;
use std::time::Duration;

use nusb::transfer::{ControlIn, ControlOut, ControlType, Recipient, TransferError};
use nusb::{DeviceInfo, Interface, MaybeFuture};

/// The USB IDs Linux's mt76x2u takes (mt76x2/usb.c), as HowlDrop for Windows
/// lists them (howldrop/src/wsl_text.hpp).
pub const USB_IDS: &[(u16, u16)] = &[
    (0x045e, 0x02e6),
    (0x045e, 0x02fe),
    (0x0471, 0x2126),
    (0x0471, 0x7600),
    (0x056e, 0x400a),
    (0x057c, 0x8503),
    (0x0846, 0x9014),
    (0x0846, 0x9053),
    (0x0b05, 0x17eb),
    (0x0b05, 0x180b),
    (0x0b05, 0x1833),
    (0x0e8d, 0x7612),
    (0x0e8d, 0x7632),
    (0x2357, 0x0137),
    (0x2c4e, 0x0103),
];

/// The EEPROM's size (MT7612U_EEPROM_SIZE).
pub const EEPROM_SIZE: usize = 512;
/// Where the MAC address is in the EEPROM (MT_EE_MAC_ADDR).
pub const EEPROM_MAC_ADDR: usize = 0x004;
/// The chip's ID and revision (MT_ASIC_VERSION): 0x7612xxxx.
pub const ASIC_VERSION: u32 = 0x0000;

/// The vendor requests that read and write registers (mt76.h, mt_vendor_req).
mod req {
    pub const MULTI_WRITE: u8 = 0x06;
    pub const MULTI_READ: u8 = 0x07;
    pub const READ_EEPROM: u8 = 0x09;
    pub const WRITE_CFG: u8 = 0x46;
    pub const READ_CFG: u8 = 0x47;
}

/// As mt76's __mt76u_vendor_request: 10 tries, 300 ms each.
const TRIES: usize = 10;
const TIMEOUT: Duration = Duration::from_millis(300);

/// The register spaces the vendor requests reach (MT_VEND_TYPE_*): the MAC's
/// own registers, the USB controller's, and the EEPROM (read only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    Mac,
    Cfg,
    Eeprom,
}

impl Space {
    fn read_request(self) -> u8 {
        match self {
            Space::Mac => req::MULTI_READ,
            Space::Cfg => req::READ_CFG,
            Space::Eeprom => req::READ_EEPROM,
        }
    }

    fn write_request(self) -> Option<u8> {
        match self {
            Space::Mac => Some(req::MULTI_WRITE),
            Space::Cfg => Some(req::WRITE_CFG),
            Space::Eeprom => None,
        }
    }
}

/// A register's address as a vendor request carries it: the high 16 bits in
/// wValue, the low 16 in wIndex.
fn split(addr: u32) -> (u16, u16) {
    ((addr >> 16) as u16, addr as u16)
}

#[derive(Debug)]
pub enum Error {
    /// No MT7612U stick is connected (or none this program may open).
    NotFound,
    Usb(nusb::Error),
    Transfer(TransferError),
    /// A register read answered with fewer than 4 bytes.
    Short {
        space: Space,
        addr: u32,
        len: usize,
    },
    /// The EEPROM can't be written through a vendor request.
    ReadOnly,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound => write!(f, "no MT7612U stick found"),
            Error::Usb(e) => write!(f, "USB: {e}"),
            Error::Transfer(e) => write!(f, "USB transfer: {e}"),
            Error::Short { space, addr, len } => {
                write!(f, "reading {space:?} {addr:#06x}: {len} bytes instead of 4")
            }
            Error::ReadOnly => write!(f, "the EEPROM is read only"),
        }
    }
}

impl std::error::Error for Error {}

impl From<nusb::Error> for Error {
    fn from(e: nusb::Error) -> Self {
        Error::Usb(e)
    }
}

impl From<TransferError> for Error {
    fn from(e: TransferError) -> Self {
        Error::Transfer(e)
    }
}

/// Whether `info` is an MT7612U stick.
pub fn is_mt7612u(info: &DeviceInfo) -> bool {
    USB_IDS.contains(&(info.vendor_id(), info.product_id()))
}

/// The MT7612U sticks connected.
pub fn sticks() -> Result<Vec<DeviceInfo>, Error> {
    Ok(nusb::list_devices().wait()?.filter(is_mt7612u).collect())
}

/// An MT7612U stick, opened.
pub struct Mt7612u {
    info: DeviceInfo,
    interface: Interface,
}

impl Mt7612u {
    /// The first stick connected.
    pub fn open() -> Result<Self, Error> {
        let info = sticks()?.into_iter().next().ok_or(Error::NotFound)?;
        Self::open_info(info)
    }

    /// This stick. On Linux it takes the stick from the kernel's mt76x2u.
    pub fn open_info(info: DeviceInfo) -> Result<Self, Error> {
        let device = info.open().wait()?;
        let interface = device.detach_and_claim_interface(0).wait()?;
        Ok(Mt7612u { info, interface })
    }

    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    /// A register (mt76u_rr).
    pub fn read(&self, space: Space, addr: u32) -> Result<u32, Error> {
        let (value, index) = split(addr);
        let data = self.retry(|| {
            self.interface
                .control_in(
                    ControlIn {
                        control_type: ControlType::Vendor,
                        recipient: Recipient::Device,
                        request: space.read_request(),
                        value,
                        index,
                        length: 4,
                    },
                    TIMEOUT,
                )
                .wait()
        })?;
        let bytes: [u8; 4] = data.as_slice().try_into().map_err(|_| Error::Short {
            space,
            addr,
            len: data.len(),
        })?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// A register (mt76u_wr).
    pub fn write(&self, space: Space, addr: u32, val: u32) -> Result<(), Error> {
        let request = space.write_request().ok_or(Error::ReadOnly)?;
        let (value, index) = split(addr);
        let data = val.to_le_bytes();
        self.retry(|| {
            self.interface
                .control_out(
                    ControlOut {
                        control_type: ControlType::Vendor,
                        recipient: Recipient::Device,
                        request,
                        value,
                        index,
                        data: &data,
                    },
                    TIMEOUT,
                )
                .wait()
        })
    }

    /// The chip's ID and revision: 0x7612 in the high half.
    pub fn asic_version(&self) -> Result<u32, Error> {
        self.read(Space::Mac, ASIC_VERSION)
    }

    /// The whole EEPROM, as mt76x2u_init_eeprom reads it: 4 bytes at a time.
    pub fn eeprom(&self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(EEPROM_SIZE);
        for addr in (0..EEPROM_SIZE as u32).step_by(4) {
            out.extend_from_slice(&self.read(Space::Eeprom, addr)?.to_le_bytes());
        }
        Ok(out)
    }

    /// As __mt76u_vendor_request: try again a few times, 5 ms apart.
    fn retry<T>(&self, mut f: impl FnMut() -> Result<T, TransferError>) -> Result<T, Error> {
        let mut last = None;
        for _ in 0..TRIES {
            match f() {
                Ok(v) => return Ok(v),
                Err(e @ TransferError::Disconnected) => return Err(e.into()),
                Err(e) => last = Some(e),
            }
            thread::sleep(Duration::from_millis(5));
        }
        Err(last.expect("tried at least once").into())
    }
}

/// The MAC address in an EEPROM image.
pub fn eeprom_mac(eeprom: &[u8]) -> Option<[u8; 6]> {
    eeprom
        .get(EEPROM_MAC_ADDR..EEPROM_MAC_ADDR + 6)?
        .try_into()
        .ok()
}

/// "00:c0:ca:ba:a7:0b".
pub fn format_mac(mac: &[u8; 6]) -> String {
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_split_as_mt76_sends_them() {
        assert_eq!(split(0x0000_1000), (0x0000, 0x1000));
        assert_eq!(split(0x0001_0234), (0x0001, 0x0234));
    }

    #[test]
    fn each_space_has_its_requests() {
        assert_eq!(Space::Mac.read_request(), 0x07);
        assert_eq!(Space::Cfg.read_request(), 0x47);
        assert_eq!(Space::Eeprom.read_request(), 0x09);
        assert_eq!(Space::Cfg.write_request(), Some(0x46));
        assert_eq!(Space::Eeprom.write_request(), None);
    }

    #[test]
    fn the_mac_address_is_at_4_in_the_eeprom() {
        let mut eeprom = vec![0u8; EEPROM_SIZE];
        eeprom[4..10].copy_from_slice(&[0x00, 0xc0, 0xca, 0xba, 0xa7, 0x0b]);
        let mac = eeprom_mac(&eeprom).unwrap();
        assert_eq!(format_mac(&mac), "00:c0:ca:ba:a7:0b");
        assert_eq!(eeprom_mac(&eeprom[..8]), None);
    }

    #[test]
    fn the_tested_stick_is_known() {
        assert!(USB_IDS.contains(&(0x0e8d, 0x7612)));
        assert_eq!(USB_IDS.len(), 15);
    }
}
