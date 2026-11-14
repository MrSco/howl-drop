//! A user-space driver for the MediaTek MT7612U (Alfa AWUS036ACM, Netgear
//! A6210 and others): enough of Linux's mt76x2u (openwrt/mt76) to run the chip
//! as a monitor-and-inject radio for owly-awdl, over nusb: WinUSB on Windows,
//! usbfs on Linux. docs/native-windows.md has the plan.
//!
//! So far: finding a stick, reading its registers and EEPROM, powering it on
//! and starting its MCU with the ROM patch and firmware (mt76x2u_init_hardware
//! up to mt76x2u_mcu_init), talking to the MCU, setting the MAC up, tuning a
//! channel and receiving every frame on it.

pub mod awdl;
pub mod channel;
pub mod mcu;
mod rate;
pub mod regs;
pub mod rx;
pub mod tx;

use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

use nusb::descriptors::TransferType;
use nusb::transfer::{
    Buffer, Bulk, ControlIn, ControlOut, ControlType, Direction, In, Out, Recipient, TransferError,
};
use nusb::{DeviceInfo, Endpoint, Interface, MaybeFuture};

use mcu::{Firmware, FirmwareError, RomPatch};

/// The USB IDs Linux's mt76x2u takes (mt76x2/usb.c): the sticks this driver
/// and HowlDrop for Windows run.
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

/// The vendor requests (mt76.h, mt_vendor_req).
mod req {
    pub const DEV_MODE: u8 = 0x01;
    pub const MULTI_WRITE: u8 = 0x06;
    pub const MULTI_READ: u8 = 0x07;
    pub const READ_EEPROM: u8 = 0x09;
    pub const WRITE_FCE: u8 = 0x42;
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
    /// The interface lacks the bulk endpoints mt76 expects (2 IN, 6 OUT).
    Endpoints,
    Firmware(FirmwareError),
    /// The chip didn't get somewhere in time.
    Timeout(&'static str),
    /// The MCU's answer wasn't the one asked for.
    Mcu(String),
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
            Error::Endpoints => write!(f, "the stick doesn't have mt76's bulk endpoints"),
            Error::Firmware(e) => write!(f, "firmware file: {e}"),
            Error::Timeout(what) => write!(f, "timed out: {what}"),
            Error::Mcu(what) => write!(f, "MCU: {what}"),
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

impl From<FirmwareError> for Error {
    fn from(e: FirmwareError) -> Self {
        Error::Firmware(e)
    }
}

/// Why [`Mt7612u::send`] didn't take a frame.
#[derive(Debug)]
pub enum SendError {
    /// The chip's queue is full: the frame, to try again.
    Full(Vec<u8>),
    /// A rate the chip doesn't have.
    Rate(tx::Rate),
}

/// Whether `info` is an MT7612U stick.
pub fn is_mt7612u(info: &DeviceInfo) -> bool {
    USB_IDS.contains(&(info.vendor_id(), info.product_id()))
}

/// The MT7612U sticks connected.
pub fn sticks() -> Result<Vec<DeviceInfo>, Error> {
    Ok(nusb::list_devices().wait()?.filter(is_mt7612u).collect())
}

/// What [`Mt7612u::start_mcu`] loaded.
pub struct Started {
    pub rom_patch_build: String,
    pub firmware_version: String,
    pub firmware_build: u16,
    pub firmware_build_time: String,
}

/// An MT7612U stick, opened.
pub struct Mt7612u {
    info: DeviceInfo,
    interface: Interface,
    /// The first bulk OUT endpoint: firmware and MCU commands
    /// (MT_EP_OUT_INBAND_CMD).
    cmd_out: Endpoint<Bulk, Out>,
    /// The second bulk IN endpoint: the MCU's responses (MT_EP_IN_CMD_RESP).
    resp_in: Endpoint<Bulk, In>,
    /// The first bulk IN endpoint: received frames (MT_EP_IN_PKT_RX).
    rx_in: Endpoint<Bulk, In>,
    /// The second bulk OUT endpoint: frames to send, best effort
    /// (MT_EP_OUT_AC_BE).
    tx_out: Endpoint<Bulk, Out>,
    /// Sent frames whose USB transfer failed.
    tx_errors: u64,
    /// The WCID table's entries from 1 on: each peer with a Block Ack
    /// session, and its TIDs' bits.
    wcids: Vec<([u8; 6], u16)>,
    /// The entry a full table reuses next.
    wcid_next: usize,
    /// The last MCU command's sequence number, 1 to 15.
    seq: u8,
    /// The TSSI compensation and DPD still to run ([`Mt7612u::calibrate`]).
    tssi: Option<Tssi>,
}

struct Tssi {
    primary: u8,
    ext_pa: bool,
    slope: [u8; 2],
    offset: [u8; 2],
    triggered: bool,
    tries: u8,
    next: Instant,
}

/// Bulk IN transfers kept in flight for received frames, and each one's size:
/// one frame each (init_dma turns aggregation off), the largest an A-MSDU.
const RX_QUEUE: usize = 8;
const RX_BUF: usize = 16 * 1024;
/// Frames handed to the chip and not yet sent; past this, [`Mt7612u::send`]
/// says the queue is full. Room for a burst of TCP ACKs: during an upload
/// the node sends 40 within a few ms, and 32 left some of them waiting for
/// the peer's next slot.
const TX_QUEUE: usize = 128;
/// Peers with Block Ack sessions at once, in WCID entries 1 on.
const MAX_WCIDS: usize = 32;

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
        // As mt76u_set_endpoints: the bulk endpoints in the order the
        // descriptor lists them, IN and OUT apart.
        let (mut ins, mut outs) = (Vec::new(), Vec::new());
        for ep in interface
            .descriptor()
            .ok_or(Error::Endpoints)?
            .endpoints()
            .filter(|ep| ep.transfer_type() == TransferType::Bulk)
        {
            match ep.direction() {
                Direction::In => ins.push(ep.address()),
                Direction::Out => outs.push(ep.address()),
            }
        }
        if ins.len() < 2 || outs.len() < 2 {
            return Err(Error::Endpoints);
        }
        let cmd_out = interface.endpoint::<Bulk, Out>(outs[0])?;
        let tx_out = interface.endpoint::<Bulk, Out>(outs[1])?;
        let resp_in = interface.endpoint::<Bulk, In>(ins[1])?;
        let rx_in = interface.endpoint::<Bulk, In>(ins[0])?;
        Ok(Mt7612u {
            info,
            interface,
            cmd_out,
            resp_in,
            rx_in,
            tx_out,
            tx_errors: 0,
            wcids: Vec::new(),
            wcid_next: 0,
            seq: 0,
            tssi: None,
        })
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
        self.request(
            ControlType::Vendor,
            request,
            value,
            index,
            &val.to_le_bytes(),
        )
    }

    /// Sets `bits` in a register (mt76_set).
    pub fn set(&self, space: Space, addr: u32, bits: u32) -> Result<(), Error> {
        let val = self.read(space, addr)?;
        self.write(space, addr, val | bits)
    }

    /// Clears `bits` in a register (mt76_clear).
    pub fn clear(&self, space: Space, addr: u32, bits: u32) -> Result<(), Error> {
        let val = self.read(space, addr)?;
        self.write(space, addr, val & !bits)
    }

    /// Reads a register until `mask` of it is `want`, for up to `timeout`
    /// (mt76_poll, mt76_poll_msec). At least a few reads, however short the
    /// time: each is a USB round trip.
    pub fn poll(
        &self,
        space: Space,
        addr: u32,
        mask: u32,
        want: u32,
        timeout: Duration,
    ) -> Result<bool, Error> {
        let until = Instant::now() + timeout;
        for tries in 0.. {
            if self.read(space, addr)? & mask == want {
                return Ok(true);
            }
            if tries >= 3 && Instant::now() >= until {
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(1));
        }
        unreachable!()
    }

    /// The chip's ID and revision: 0x7612 in the high half.
    pub fn asic_version(&self) -> Result<u32, Error> {
        self.read(Space::Mac, regs::ASIC_VERSION)
    }

    /// The whole EEPROM, as mt76x2u_init_eeprom reads it: 4 bytes at a time.
    pub fn eeprom(&self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(EEPROM_SIZE);
        for addr in (0..EEPROM_SIZE as u32).step_by(4) {
            out.extend_from_slice(&self.read(Space::Eeprom, addr)?.to_le_bytes());
        }
        Ok(out)
    }

    /// Powers the chip on and starts its MCU with the ROM patch
    /// (mt7662_rom_patch.bin) and the firmware (mt7662.bin): what
    /// mt76x2u_init_hardware does up to mt76x2u_mcu_init.
    pub fn start_mcu(&mut self, rom_patch: &[u8], firmware: &[u8]) -> Result<Started, Error> {
        let rom_patch = RomPatch::parse(rom_patch)?;
        let firmware = Firmware::parse(firmware)?;
        let rev = self.asic_version()? & 0xffff;
        let drained = self.quiesce()?;
        if drained > 0 {
            tracing::debug!("{drained} transfers left from a run before, read and dropped");
        }

        let t = Instant::now();
        let stage = |what: &str| tracing::debug!("{what}: {} ms", t.elapsed().as_millis());
        self.reset_wlan()?;
        self.power_on()?;
        stage("powered on");
        self.wait_for_mac()?;
        stage("MAC up");
        self.load_rom_patch(&rom_patch, rev)?;
        stage("ROM patch applied");
        self.load_firmware(&firmware, rev)?;
        stage("firmware running");
        let busy = regs::WPDMA_GLO_CFG_TX_DMA_BUSY | regs::WPDMA_GLO_CFG_RX_DMA_BUSY;
        if !self.poll(Space::Mac, regs::WPDMA_GLO_CFG, busy, 0, ms(100))? {
            return Err(Error::Timeout("the DMA to go idle after the firmware"));
        }
        self.wait_for_mac()?;
        stage("MAC up again");
        self.init_dma()?;
        // mt76x2u_mcu_init: responses on the command queue, the radio on.
        let mut q_select = mcu::Q_SELECT.to_le_bytes().to_vec();
        q_select.extend_from_slice(&1u32.to_le_bytes());
        self.mcu_send(mcu::cmd::FUN_SET_OP, &q_select, false)?;
        let mut radio = mcu::RADIO_ON.to_le_bytes().to_vec();
        radio.extend_from_slice(&0u32.to_le_bytes());
        self.mcu_send(mcu::cmd::POWER_SAVING_OP, &radio, false)?;

        Ok(Started {
            rom_patch_build: rom_patch.build_time,
            firmware_version: firmware.version(),
            firmware_build: firmware.build_ver,
            firmware_build_time: firmware.build_time,
        })
    }

    /// Registers read by the MCU (CMD_RANDOM_READ): `base` plus each of
    /// `regs`, up to 24 at a time.
    pub fn mcu_read(&mut self, base: u32, regs: &[u32]) -> Result<Vec<u32>, Error> {
        if regs.len() > mcu::INBAND_MAX / 8 {
            return Err(Error::Mcu(format!("{} registers in one read", regs.len())));
        }
        let response = self
            .mcu_send(mcu::cmd::RANDOM_READ, &mcu::random_read(base, regs), true)?
            .unwrap_or_default();
        mcu::random_read_values(&response, base, regs)
            .ok_or_else(|| Error::Mcu("its random read answered other registers".into()))
    }

    /// Registers written by the MCU (CMD_RANDOM_WRITE, mt76x02u_mcu_wr_rp):
    /// `base` plus each address, 24 to a command, the last one answered.
    pub fn mcu_write(&mut self, base: u32, pairs: &[(u32, u32)]) -> Result<(), Error> {
        let chunks: Vec<_> = pairs.chunks(mcu::INBAND_MAX / 8).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let payload: Vec<u8> = chunk
                .iter()
                .flat_map(|&(reg, val)| mcu::words(base + reg, val))
                .collect();
            self.mcu_send(mcu::cmd::RANDOM_WRITE, &payload, i + 1 == chunks.len())?;
        }
        Ok(())
    }

    /// Takes the frames a peer aggregates on `tid` with a Block Ack from the
    /// chip, or stops (mt76x02_ampdu_action, RX_START and RX_STOP): the
    /// peer's entry in the WCID table, its bit for `tid` set or cleared.
    /// `peer`'s WCID entry, made if it has none, for frames sent to it
    /// under a Block Ack session: the chip matches the peer's Block Acks to
    /// it. The index the TXWI takes.
    pub fn tx_wcid(&mut self, peer: [u8; 6]) -> Result<u8, Error> {
        let idx = match self.wcids.iter().position(|(mac, _)| *mac == peer) {
            Some(i) => return Ok(i as u8 + 1),
            None if self.wcids.len() < MAX_WCIDS => {
                self.wcids.push((peer, 0));
                self.wcids.len() - 1
            }
            None => {
                let i = self.wcid_next % MAX_WCIDS;
                self.wcid_next += 1;
                self.wcids[i] = (peer, 0);
                i
            }
        };
        let [w0, w1] = regs::wcid_words(peer, 0);
        let at = regs::wcid_addr(idx as u32 + 1);
        self.write(Space::Mac, at, w0)?;
        self.write(Space::Mac, at + 4, w1)?;
        Ok(idx as u8 + 1)
    }

    pub fn rx_block_ack(&mut self, peer: [u8; 6], tid: u8, on: bool) -> Result<(), Error> {
        let bit = 1u16 << (tid & 0xf);
        let idx = match self.wcids.iter().position(|(mac, _)| *mac == peer) {
            Some(i) => i,
            None if !on => return Ok(()),
            None if self.wcids.len() < MAX_WCIDS => {
                self.wcids.push((peer, 0));
                self.wcids.len() - 1
            }
            // Full: the next entry round makes room, in place, so the
            // others keep their slots.
            None => {
                let i = self.wcid_next % MAX_WCIDS;
                self.wcid_next += 1;
                self.wcids[i] = (peer, 0);
                i
            }
        };
        let mask = &mut self.wcids[idx].1;
        if on {
            *mask |= bit;
        } else {
            *mask &= !bit;
        }
        let [w0, w1] = regs::wcid_words(peer, *mask);
        // Entry 0 is left alone, as mt76 keeps it.
        let at = regs::wcid_addr(idx as u32 + 1);
        self.write(Space::Mac, at, w0)?;
        self.write(Space::Mac, at + 4, w1)
    }

    /// The WCID table emptied (as mt76x2u_init_hardware resets it): every
    /// station's attributes, and the addresses and Block Ack bits of the
    /// first 128.
    fn reset_wcids(&mut self) -> Result<(), Error> {
        let mut pairs: Vec<(u32, u32)> = (0..256).map(|i| (regs::wcid_attr(i), 0)).collect();
        for i in 0..128 {
            pairs.push((regs::wcid_addr(i), 0));
            pairs.push((regs::wcid_addr(i) + 4, 0));
        }
        self.mcu_write(regs::MCU_MEMMAP_WLAN, &pairs)?;
        self.wcids.clear();
        self.wcid_next = 0;
        Ok(())
    }

    /// The rest of mt76x2u_init_hardware once the MCU runs: the MAC's
    /// registers (mt76x2u_mac_reset), its address, the RF's and baseband's
    /// registers from the MCU (LOAD_CR), and both receive and transmit
    /// chains. The MAC is left stopped. `eeprom` is [`Mt7612u::eeprom`]'s.
    pub fn init_mac(&mut self, eeprom: &[u8]) -> Result<(), Error> {
        use Space::Mac;
        let ee = |off| ee16(eeprom, off);

        // mt76x2u_mac_reset
        self.write(Mac, regs::WPDMA_GLO_CFG, (1 << 4) | (1 << 5))?;
        self.write(Mac, regs::PBF_TX_MAX_PCNT, 0xefef_3f1f)?;
        self.write(Mac, regs::PBF_RX_MAX_PCNT, 0xfebf)?;
        for &(reg, val) in MAC_INITVALS {
            self.write(Mac, reg, val)?;
        }
        self.write(Mac, regs::TX_LINK_CFG, 0x1020)?;
        self.write(Mac, regs::AUTO_RSP_CFG, 0x13)?;
        self.write(Mac, regs::MAX_LEN_CFG, 0x2f00)?;
        self.write(Mac, regs::WMM_AIFSN, 0x2273)?;
        self.write(Mac, regs::WMM_CWMIN, 0x2344)?;
        self.write(Mac, regs::WMM_CWMAX, 0x34aa)?;
        self.clear(
            Mac,
            regs::MAC_SYS_CTRL,
            regs::MAC_SYS_CTRL_RESET_CSR | regs::MAC_SYS_CTRL_RESET_BBP,
        )?;
        self.clear(Mac, regs::COEXCFG0, regs::COEXCFG0_COEX_EN)?;
        self.set(Mac, regs::EXT_CCA_CFG, 0xf000)?;
        self.clear(Mac, regs::TX_ALC_CFG_4, 1 << 31)?;
        self.fixup_xtal(eeprom)?;

        // mt76x02_mac_setaddr, with the EEPROM's address
        let mac = eeprom_mac(eeprom).ok_or(Error::Mcu("no MAC address in the EEPROM".into()))?;
        let dw0 = u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]);
        let dw1 = u32::from(u16::from_le_bytes([mac[4], mac[5]]));
        self.write(Mac, regs::MAC_ADDR_DW0, dw0)?;
        self.write(Mac, regs::MAC_ADDR_DW1, dw1 | regs::MAC_ADDR_DW1_U2ME_MASK)?;
        self.write(Mac, regs::MAC_BSSID_DW0, dw0)?;
        // 8 APs + 8 STAs, the local bit, 7 more beacon slots
        self.write(
            Mac,
            regs::MAC_BSSID_DW1,
            dw1 | (3 << 16) | (1 << 21) | (7 << 18),
        )?;

        self.rmw(Mac, regs::US_CYC_CFG, regs::US_CYC_CNT, 0x1e)?;
        self.write(Mac, regs::TXOP_CTRL_CFG, 0x583f)?;

        // mt76x2_mcu_load_cr(dev, MT_RF_BBP_CR, 0, 0)
        let cfg = (1 << 31)
            | ((u32::from(ee(regs::ee::NIC_CONF_0)) >> 8) & 0x00ff)
            | ((u32::from(ee(regs::ee::NIC_CONF_1)) << 8) & 0xff00);
        let mut load_cr = vec![mcu::RF_BBP_CR, 0, 0, 0];
        load_cr.extend_from_slice(&cfg.to_le_bytes());
        self.mcu_send(mcu::cmd::LOAD_CR, &load_cr, true)?;

        // mt76x02_phy_set_rxpath and _set_txdac, two chains each (0x202)
        let agc0 = self.read(Mac, regs::bbp(regs::BBP_AGC, 0))?;
        self.write(
            Mac,
            regs::bbp(regs::BBP_AGC, 0),
            (agc0 & !(1 << 4)) | (1 << 3),
        )?;
        self.set(Mac, regs::bbp(regs::BBP_TXBE, 5), 0x3)?;

        // reset wcid table
        self.reset_wcids()?;

        self.clear(
            Mac,
            regs::MAC_SYS_CTRL,
            regs::MAC_SYS_CTRL_ENABLE_TX | regs::MAC_SYS_CTRL_ENABLE_RX,
        )
    }

    /// mt76x2u_phy_set_channel, with the MAC stopped. Not yet: the EEPROM's
    /// receive gain corrections (the MCU's gain is left at 0).
    pub fn set_channel(&mut self, eeprom: &[u8], ch: channel::Channel) -> Result<(), Error> {
        use Space::Mac;
        let t = ch.tuning();
        let five = u32::from(ch.is_5ghz());
        let rev = self.asic_version()? & 0xffff;

        self.set_txpower(eeprom, ch)?;

        // mt76x02_phy_set_band
        let (on, off) = if ch.is_5ghz() {
            (regs::TX_BAND_CFG_5G, regs::TX_BAND_CFG_2G)
        } else {
            (regs::TX_BAND_CFG_2G, regs::TX_BAND_CFG_5G)
        };
        self.clear(Mac, regs::TX_BAND_CFG, off)?;
        self.set(Mac, regs::TX_BAND_CFG, on)?;
        self.rmw_field(
            Mac,
            regs::TX_BAND_CFG,
            regs::TX_BAND_CFG_UPPER_40M,
            u32::from(t.group & 1),
        )?;

        // mt76x02_phy_set_bw
        let (core, agc) = match ch.width {
            channel::Width::Ht20 => (0, 1),
            _ => (2, 3),
        };
        let group = u32::from(t.group);
        self.rmw_field(
            Mac,
            regs::bbp(regs::BBP_CORE, 1),
            regs::BBP_CORE_R1_BW,
            core,
        )?;
        self.rmw_field(Mac, regs::bbp(regs::BBP_AGC, 0), regs::BBP_AGC_R0_BW, agc)?;
        self.rmw_field(
            Mac,
            regs::bbp(regs::BBP_AGC, 0),
            regs::BBP_AGC_R0_CTRL_CHAN,
            group,
        )?;
        self.rmw_field(
            Mac,
            regs::bbp(regs::BBP_TXBE, 0),
            regs::BBP_TXBE_R0_CTRL_CHAN,
            group,
        )?;
        self.rmw(
            Mac,
            regs::EXT_CCA_CFG,
            regs::EXT_CCA_CFG_CCA_FIELDS,
            ch.ext_cca(),
        )?;

        // mt76x2_mcu_set_channel: first without the extension channel
        let mut switch = vec![t.center, 0, t.bw, 0];
        switch.extend_from_slice(&0x0202u16.to_le_bytes()); // chainmask
        switch.extend_from_slice(&[0, 0]);
        self.mcu_send(mcu::cmd::SWITCH_CHANNEL_OP, &switch, true)?;
        thread::sleep(ms(10));
        switch[6] = 0xe0 + t.bw_index;
        self.mcu_send(mcu::cmd::SWITCH_CHANNEL_OP, &switch, true)?;

        // mt76x2_mcu_init_gain(dev, channel, gain, true)
        let gain = 0;
        self.mcu_send(
            mcu::cmd::INIT_GAIN_OP,
            &mcu::words(u32::from(t.center) | (1 << 31), gain),
            true,
        )?;

        // Enable LDPC Rx
        if rev >= regs::REV_E3 {
            self.set(Mac, regs::bbp(regs::BBP_RXO, 13), 1 << 10)?;
        }
        if ee16(eeprom, regs::ee::BT_RCAL_RESULT) & 0xff != 0xff {
            self.mcu_calibrate(mcu::cal::R, 0)?;
        }
        self.mcu_calibrate(mcu::cal::RXDCOC, u32::from(t.center))?;
        // Rx LPF calibration
        self.mcu_calibrate(mcu::cal::RC, 0)?;

        self.write(Mac, regs::bbp(regs::BBP_AGC, 61), 0xff64_a4e2)?;
        self.write(Mac, regs::bbp(regs::BBP_AGC, 7), 0x0808_1010)?;
        self.write(Mac, regs::bbp(regs::BBP_AGC, 11), 0x0000_0404)?;
        self.write(Mac, regs::bbp(regs::BBP_AGC, 2), 0x0000_7070)?;
        self.write(Mac, regs::TXOP_CTRL_CFG, 0x0410_1b3f)?;
        self.set(Mac, regs::bbp(regs::BBP_TXO, 4), 1 << 25)?;
        self.set(Mac, regs::bbp(regs::BBP_RXO, 13), 1 << 8)?;

        // mt76x2u_phy_channel_calibrate(dev, true)
        if ch.is_5ghz() {
            self.mcu_calibrate(mcu::cal::LC, 0)?;
        }
        self.mcu_calibrate(mcu::cal::TX_LOFT, five)?;
        self.mcu_calibrate(mcu::cal::TXIQ, five)?;
        self.mcu_calibrate(mcu::cal::RXIQC_FI, five)?;
        self.mcu_calibrate(mcu::cal::TEMP_SENSOR, 0)?;
        self.mcu_calibrate(mcu::cal::TX_SHAPING, 0)?;

        // mt76x02_edcca_init, energy detection off
        self.set(Mac, regs::TX_LINK_CFG, 1 << 12)?;
        self.clear(Mac, regs::TXOP_CTRL_CFG, 1 << 20)?;
        self.write(Mac, regs::bbp(regs::BBP_AGC, 2), 0x0000_7070)?;
        self.set(Mac, regs::TXOP_HLDR_ET, 1 << 1)?;

        // mt76x2u_phy_set_channel's TSSI calibration, with temperature
        // compensation at its default. The compensation and DPD that follow
        // need frames going out: see [`Mt7612u::calibrate`].
        self.tssi = None;
        if channel::tssi_enabled(eeprom) {
            let ext_pa = ee16(eeprom, regs::ee::NIC_CONF_0) & regs::ee::NIC_CONF_0_PA_INT_5G == 0;
            for reg in [regs::TX_ALC_CFG_1, regs::TX_ALC_CFG_2] {
                self.rmw_field(Mac, reg, regs::TX_ALC_CFG_TEMP_COMP, 0x38)?;
            }
            let flag = five | if ext_pa { 1 << 8 } else { 0 };
            self.mcu_calibrate(mcu::cal::TSSI, flag)?;
            let (slope, offset) = channel::tssi_slope_offset_5g(eeprom, ch.primary);
            self.tssi = Some(Tssi {
                primary: ch.primary,
                ext_pa,
                slope,
                offset,
                triggered: false,
                tries: 0,
                next: Instant::now() + Duration::from_secs(2),
            });
        }
        Ok(())
    }

    /// What mt76x2u_phy_calibrate does after a channel is set
    /// (mt76x2_phy_tssi_compensate), once: a TSSI measurement of frames
    /// going out, applied once the baseband has it, and then, on a chip
    /// with its own power amplifier, DPD, which predistorts what it sends
    /// against the amplifier's distortion. Without them 54 Mbit/s and MCS 7
    /// frames needed some two retries each at 1 m. Linux repeats the
    /// compensation every second; once is what the rates need. The commands
    /// go unanswered: waiting for the MCU while the radio is busy held up
    /// the node, and the answer sometimes never came.
    /// Call it often once frames are going out; it acts every second until
    /// it's done.
    pub fn calibrate(&mut self) -> Result<(), Error> {
        use Space::Mac;
        let Some(t) = self.tssi.as_mut() else {
            return Ok(());
        };
        let now = Instant::now();
        if now < t.next {
            return Ok(());
        }
        t.next = now + Duration::from_secs(1);
        if !t.triggered {
            t.triggered = true;
            return self.tssi_comp(0, 1, [0; 2], [0; 2]);
        }
        if self.read(Mac, regs::bbp(regs::BBP_CORE, 34))? & (1 << 4) != 0 {
            let t = self.tssi.as_mut().expect("checked above");
            t.tries += 1;
            if t.tries >= 5 {
                self.tssi = None;
                tracing::info!(target: "owly_awdl", "TSSI not measured; no DPD");
            }
            return Ok(());
        }
        let t = self.tssi.take().expect("checked above");
        self.tssi_comp(u8::from(t.ext_pa), 2, t.slope, t.offset)?;
        if !t.ext_pa {
            thread::sleep(ms(15));
            let dpd = mcu::words(mcu::cal::DPD, u32::from(t.primary));
            self.mcu_send(mcu::cmd::CALIBRATION_OP, &dpd, false)?;
        }
        tracing::info!(
            target: "owly_awdl",
            "TSSI compensated{} on ch {}",
            if t.ext_pa { "" } else { " and DPD calibrated" },
            t.primary
        );
        Ok(())
    }

    /// mt76x2_mcu_tssi_comp.
    fn tssi_comp(
        &mut self,
        pa_mode: u8,
        cal_mode: u8,
        slope: [u8; 2],
        offset: [u8; 2],
    ) -> Result<(), Error> {
        let mut msg = mcu::cal::TSSI_COMP.to_le_bytes().to_vec();
        msg.extend_from_slice(&[
            pa_mode, cal_mode, 0, 0, slope[0], slope[1], offset[0], offset[1],
        ]);
        // Unanswered: the baseband says when the measurement is in, and an
        // answer awaited while the radio is busy could hold up the node for
        // 1.5 s, or not come.
        self.mcu_send(mcu::cmd::CALIBRATION_OP, &msg, false)?;
        Ok(())
    }

    /// mt76x02u_mac_start with a filter that keeps every frame but broken
    /// ones and control frames other than Block Acks, then bulk IN transfers
    /// queued for them.
    pub fn start_rx(&mut self) -> Result<(), Error> {
        use Space::Mac;
        use regs::rx_filter::*;
        let busy = regs::WPDMA_GLO_CFG_TX_DMA_BUSY | regs::WPDMA_GLO_CFG_RX_DMA_BUSY;
        self.write(Mac, regs::MAC_SYS_CTRL, regs::MAC_SYS_CTRL_ENABLE_TX)?;
        if !self.poll(Mac, regs::WPDMA_GLO_CFG, busy, 0, ms(200))? {
            return Err(Error::Timeout("the DMA to go idle"));
        }
        let drop =
            CRC_ERR | PHY_ERR | VER_ERR | DUP | CFACK | CFEND | ACK | CTS | RTS | PSPOLL | CTRL_RSV;
        self.write(Mac, regs::RX_FILTR_CFG, drop)?;
        self.write(
            Mac,
            regs::MAC_SYS_CTRL,
            regs::MAC_SYS_CTRL_ENABLE_TX | regs::MAC_SYS_CTRL_ENABLE_RX,
        )?;
        if !self.poll(Mac, regs::WPDMA_GLO_CFG, busy, 0, ms(50))? {
            return Err(Error::Timeout("the DMA to go idle"));
        }
        while self.rx_in.pending() < RX_QUEUE {
            self.rx_in.submit(Buffer::new(RX_BUF));
        }
        Ok(())
    }

    /// The next frame received, waiting up to `timeout`; None when none came.
    pub fn next_frame(&mut self, timeout: Duration) -> Result<Option<rx::RxFrame>, Error> {
        let until = Instant::now() + timeout;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let Some(done) = self.rx_in.wait_next_complete(left) else {
                return Ok(None);
            };
            self.rx_in.submit(Buffer::new(RX_BUF));
            done.status?;
            if let Some(frame) = rx::parse(&done.buffer[..done.actual_len]) {
                return Ok(Some(frame));
            }
        }
    }

    /// Hands `frame` (from its MAC header, without FCS) to the chip at `rate`,
    /// without waiting. Gives it back when the chip's queue is full, and
    /// fails for a rate the chip doesn't have.
    pub fn send(&mut self, frame: Vec<u8>, rate: tx::Rate, no_ack: bool) -> Result<(), SendError> {
        self.send_with(frame, rate, no_ack, None)
    }

    /// As [`Mt7612u::send`], marked for an A-MPDU when `ampdu` is given.
    pub fn send_with(
        &mut self,
        frame: Vec<u8>,
        rate: tx::Rate,
        no_ack: bool,
        ampdu: Option<tx::Ampdu>,
    ) -> Result<(), SendError> {
        self.reap_tx();
        if self.tx_out.pending() >= TX_QUEUE {
            return Err(SendError::Full(frame));
        }
        let rate = rate.value().ok_or(SendError::Rate(rate))?;
        self.tx_out
            .submit(Buffer::from(tx::packet_with(&frame, rate, no_ack, ampdu)));
        Ok(())
    }

    /// How many sent frames the USB transfer failed for.
    pub fn tx_errors(&self) -> u64 {
        self.tx_errors
    }

    /// Collects the USB transfers that have completed, without waiting.
    pub fn reap_tx(&mut self) {
        while self.tx_out.pending() > 0 {
            match self.tx_out.wait_next_complete(Duration::ZERO) {
                Some(done) => self.tx_errors += u64::from(done.status.is_err()),
                None => break,
            }
        }
    }

    /// Frames handed to the chip whose USB transfer hasn't completed.
    pub fn tx_pending(&self) -> usize {
        self.tx_out.pending()
    }

    /// Waits up to `timeout` for the frames handed to the chip to be taken;
    /// how many are left.
    pub fn flush_tx(&mut self, timeout: Duration) -> usize {
        let until = Instant::now() + timeout;
        while self.tx_out.pending() > 0 {
            let left = until.saturating_duration_since(Instant::now());
            match self.tx_out.wait_next_complete(left) {
                Some(done) => self.tx_errors += u64::from(done.status.is_err()),
                None => break,
            }
        }
        self.tx_out.pending()
    }

    /// Transmit power, as mt76x2_phy_set_txpower: the PA and delay registers
    /// for the band (mt76x2_phy_set_txpower_regs, mt76x2_configure_tx_delay),
    /// each chain's power for the channel's calibration group in the EEPROM,
    /// and each rate's own power from it.
    fn set_txpower(&self, eeprom: &[u8], ch: channel::Channel) -> Result<(), Error> {
        use Space::Mac;
        if !ch.is_5ghz() {
            return Err(Error::Mcu(
                "transmit power is only set up for 5 GHz yet".into(),
            ));
        }
        let ext_pa = ee16(eeprom, regs::ee::NIC_CONF_0) & regs::ee::NIC_CONF_0_PA_INT_5G == 0;
        let bw40 = !matches!(ch.width, channel::Width::Ht20);

        // mt76x2_configure_tx_delay
        let (cfg0, cfg1) = match (ext_pa, bw40) {
            (true, true) => (0x000b_0c01, 0x0001_1414),
            (true, false) => (0x0010_1101, 0x0001_1414),
            (false, true) => (0x000b_0b01, 0x0002_1414),
            (false, false) => (0x0010_1001, 0x0002_1414),
        };
        self.write(Mac, regs::TX_SW_CFG0, cfg0)?;
        self.write(Mac, regs::TX_SW_CFG1, cfg1)?;
        self.rmw_field(Mac, regs::XIFS_TIME_CFG, regs::XIFS_TIME_CFG_OFDM_SIFS, 15)?;

        // mt76x2_phy_set_txpower_regs, 5 GHz
        let (alc2, alc3, pa_adj) = if ext_pa {
            (0x2f0f_0400, 0x2f0f_0476, 0x0400_0000)
        } else {
            (0x1b0f_0400, 0x1b0f_0476, 0)
        };
        self.write(Mac, regs::TX_ALC_CFG_2, alc2)?;
        self.write(Mac, regs::TX_ALC_CFG_3, alc3)?;
        self.write(Mac, regs::RF_PA_MODE_ADJ0, pa_adj)?;
        self.write(Mac, regs::RF_PA_MODE_ADJ1, pa_adj)?;
        self.write(Mac, regs::BB_PA_MODE_CFG0, 0x0000_ffff)?;
        self.write(Mac, regs::BB_PA_MODE_CFG1, 0x00ff_00ff)?;
        self.write(Mac, regs::RF_PA_MODE_CFG0, 0x0000_ffff)?;
        self.write(Mac, regs::RF_PA_MODE_CFG1, 0x00ff_00ff)?;
        if ext_pa {
            self.write(Mac, regs::TX0_RF_GAIN_CORR, 0x363c_023c)?;
            self.write(Mac, regs::TX1_RF_GAIN_CORR, 0x363c_023c)?;
            self.write(Mac, regs::TX_ALC_CFG_4, 0x0000_1818)?;
        } else {
            self.write(Mac, regs::TX0_RF_GAIN_CORR, 0x383c_023c)?;
            self.write(Mac, regs::TX1_RF_GAIN_CORR, 0x2428_2e28)?;
            self.write(Mac, regs::TX_ALC_CFG_4, 0)?;
        }

        // Each chain's power, and each rate's offset from it
        // (mt76x2_phy_set_txpower).
        let power = channel::tx_power_5g(eeprom, ch.primary, bw40);
        let [txp_0, txp_1] = power.chain;
        tracing::info!(
            target: "owly_awdl",
            "transmit power {txp_0:#04x} and {txp_1:#04x}, rate offsets {:08x?}, {} PA",
            power.rate_cfg,
            if ext_pa { "external" } else { "internal" }
        );
        self.rmw_field(Mac, regs::TX_ALC_CFG_0, regs::TX_ALC_CFG_0_CH_INIT_0, txp_0)?;
        self.rmw_field(Mac, regs::TX_ALC_CFG_0, regs::TX_ALC_CFG_0_CH_INIT_1, txp_1)?;
        let rate_regs = [
            regs::TX_PWR_CFG_0,
            regs::TX_PWR_CFG_1,
            regs::TX_PWR_CFG_2,
            regs::TX_PWR_CFG_3,
            regs::TX_PWR_CFG_4,
            regs::TX_PWR_CFG_7,
            regs::TX_PWR_CFG_8,
            regs::TX_PWR_CFG_9,
        ];
        for (reg, val) in rate_regs.into_iter().zip(power.rate_cfg) {
            self.write(Mac, reg, val)?;
        }
        Ok(())
    }

    /// mt76x02_mcu_calibrate.
    fn mcu_calibrate(&mut self, kind: u32, param: u32) -> Result<(), Error> {
        self.mcu_send(mcu::cmd::CALIBRATION_OP, &mcu::words(kind, param), true)?;
        Ok(())
    }

    /// mt76x2u_mac_fixup_xtal: the crystal's trim from the EEPROM, and the
    /// timing fix-ups that come with it.
    fn fixup_xtal(&self, eeprom: &[u8]) -> Result<(), Error> {
        use Space::{Cfg, Mac};
        let trim2 = ee16(eeprom, regs::ee::XTAL_TRIM_2);
        let mut offset = i32::from(trim2 & 0x7f);
        if trim2 & 0xff == 0xff {
            offset = 0;
        } else if trim2 & 0x80 != 0 {
            offset = -offset;
        }
        let mut val = trim2 >> 8;
        if val == 0x00 || val == 0xff {
            val = ee16(eeprom, regs::ee::XTAL_TRIM_1) & 0xff;
            if val == 0x00 || val == 0xff {
                val = 0x14;
            }
        }
        let c2 = (i32::from(val & 0x7f) + offset) as u32;
        self.rmw_field(Cfg, regs::XO_CTRL5, regs::XO_CTRL5_C2_VAL, c2)?;
        self.set(Cfg, regs::XO_CTRL6, regs::XO_CTRL6_C2_CTRL)?;

        self.write(Mac, 0x504, 0x0600_0000)?;
        self.write(Mac, 0x50c, 0x0880_0000)?;
        thread::sleep(ms(5));
        self.write(Mac, 0x504, 0)?;

        // decrease SIFS from 16us to 13us
        self.rmw_field(Mac, regs::XIFS_TIME_CFG, regs::XIFS_TIME_CFG_OFDM_SIFS, 0xd)?;
        self.rmw_field(Mac, regs::BKOFF_SLOT_CFG, regs::BKOFF_SLOT_CFG_CC_DELAY, 1)?;
        // init fce
        self.clear(Mac, regs::FCE_L2_STUFF, regs::FCE_L2_STUFF_WR_MPDU_LEN_EN)?;

        let option = (ee16(eeprom, regs::ee::NIC_CONF_2) & regs::ee::NIC_CONF_2_XTAL_OPTION) >> 9;
        match option {
            0 => self.write(Mac, regs::XO_CTRL7, 0x5c1f_ee80),
            1 => self.write(Mac, regs::XO_CTRL7, 0x5c1f_eed0),
            _ => Ok(()),
        }
    }

    /// Replaces `mask` of a register with `val` (mt76_rmw).
    fn rmw(&self, space: Space, addr: u32, mask: u32, val: u32) -> Result<(), Error> {
        let old = self.read(space, addr)?;
        self.write(space, addr, (old & !mask) | (val & mask))
    }

    /// Sets the field `mask` covers to `val` (mt76_rmw_field).
    fn rmw_field(&self, space: Space, addr: u32, mask: u32, val: u32) -> Result<(), Error> {
        self.rmw(space, addr, mask, val << mask.trailing_zeros())
    }

    /// A command to the MCU (__mt76x02u_mcu_send_msg). With `wait`, its
    /// response: the MCU's answer to this sequence number, event "done".
    pub fn mcu_send(
        &mut self,
        cmd: u8,
        payload: &[u8],
        wait: bool,
    ) -> Result<Option<Vec<u8>>, Error> {
        let seq = if wait {
            self.seq = self.seq % 15 + 1;
            self.seq
        } else {
            0
        };
        self.bulk_out(mcu::command(cmd, seq, payload), ms(500))?;
        if !wait {
            return Ok(None);
        }
        // mt76x02u_mcu_wait_resp: five reads of 300 ms.
        let mut heard = None;
        for _ in 0..5 {
            let done = self
                .resp_in
                .transfer_blocking(Buffer::new(mcu::RESP_SIZE), ms(300));
            match done.status {
                Err(TransferError::Cancelled) => continue,
                Err(e) => return Err(e.into()),
                Ok(()) => {}
            }
            let data = done.buffer[..done.actual_len].to_vec();
            match mcu::response(&data) {
                Some(r) if r.seq == seq && r.event == 0 => return Ok(Some(data)),
                other => heard = Some(other),
            }
        }
        Err(match heard {
            Some(r) => Error::Mcu(format!("command {cmd} (seq {seq}): answered {r:?}")),
            None => Error::Timeout("the MCU's response"),
        })
    }

    /// A stick a process before left running, made quiet: the MAC stopped,
    /// then what waits on the bulk IN endpoints read and dropped. Linux
    /// resets the whole device instead (usb_reset_device in mt76x2u_probe),
    /// as does usbip's attach; WinUSB can't, and unread frames otherwise
    /// hold up the MCU's answers. How many transfers it dropped.
    fn quiesce(&mut self) -> Result<usize, Error> {
        self.write(Space::Mac, regs::MAC_SYS_CTRL, 0)?;
        let mut dropped = 0;
        for ep in [&mut self.rx_in, &mut self.resp_in] {
            for _ in 0..256 {
                let done = ep.transfer_blocking(Buffer::new(RX_BUF), ms(20));
                if done.status.is_err() {
                    break;
                }
                dropped += 1;
            }
        }
        Ok(dropped)
    }

    /// mt76x2_reset_wlan(dev, true).
    fn reset_wlan(&self) -> Result<(), Error> {
        let mut val = self.read(Space::Mac, regs::WLAN_FUN_CTRL)?;
        val &= !regs::WLAN_FUN_CTRL_FRC_WL_ANT_SEL;
        if val & regs::WLAN_FUN_CTRL_WLAN_EN != 0 {
            self.write(
                Space::Mac,
                regs::WLAN_FUN_CTRL,
                val | regs::WLAN_FUN_CTRL_WLAN_RESET_RF,
            )?;
            val &= !regs::WLAN_FUN_CTRL_WLAN_RESET_RF;
        }
        self.write(Space::Mac, regs::WLAN_FUN_CTRL, val)?;
        // mt76x2_set_wlan_state(dev, true)
        self.set(
            Space::Mac,
            regs::WLAN_FUN_CTRL,
            regs::WLAN_FUN_CTRL_WLAN_EN | regs::WLAN_FUN_CTRL_WLAN_CLK_EN,
        )
    }

    /// mt76x2u_power_on.
    fn power_on(&self) -> Result<(), Error> {
        use Space::Cfg;
        // Turn on WL MTCMOS
        self.set(Cfg, regs::WLAN_MTC_CTRL, regs::WLAN_MTC_CTRL_MTCMOS_PWR_UP)?;
        let up = regs::WLAN_MTC_CTRL_STATE_UP
            | regs::WLAN_MTC_CTRL_PWR_ACK
            | regs::WLAN_MTC_CTRL_PWR_ACK_S;
        // mt76 doesn't check this one either.
        self.poll(Cfg, regs::WLAN_MTC_CTRL, up, up, ms(10))?;
        self.clear(Cfg, regs::WLAN_MTC_CTRL, 0x7f << 16)?;
        self.clear(Cfg, regs::WLAN_MTC_CTRL, 0xf << 24)?;
        self.set(Cfg, regs::WLAN_MTC_CTRL, 0xf << 24)?;
        self.clear(Cfg, regs::WLAN_MTC_CTRL, 0xfff)?;
        // Turn on AD/DA power down
        self.clear(Cfg, 0x1204, 1 << 3)?;
        // WLAN function enable
        self.set(Cfg, 0x80, 1 << 0)?;
        // Release BBP software reset
        self.clear(Cfg, 0x64, 1 << 18)?;
        self.power_on_rf(0)?;
        self.power_on_rf(1)
    }

    /// mt76x2u_power_on_rf.
    fn power_on_rf(&self, unit: u32) -> Result<(), Error> {
        use Space::Cfg;
        let shift = if unit == 0 { 0 } else { 8 };
        // Enable RF BG
        self.set(Cfg, 0x130, 1 << shift)?;
        // Enable RFDIG LDO/AFE/ABB/ADDA
        self.set(
            Cfg,
            0x130,
            ((1 << 1) | (1 << 3) | (1 << 4) | (1 << 5)) << shift,
        )?;
        // Switch RFDIG power to internal LDO
        self.clear(Cfg, 0x130, (1 << 2) << shift)?;
        // mt76x2u_power_on_rf_patch
        self.set(Cfg, 0x130, (1 << 0) | (1 << 16))?;
        self.clear(Cfg, 0x1c, 0xff)?;
        self.set(Cfg, 0x1c, 0x30)?;
        self.write(Cfg, 0x14, 0x484f)?;
        self.set(Cfg, 0x130, 1 << 17)?;
        self.clear(Cfg, 0x130, 1 << 16)?;
        self.set(Cfg, 0x14c, (1 << 19) | (1 << 20))?;

        self.set(Space::Mac, 0x530, 0xf)
    }

    /// mt76x02_wait_for_mac: MAC_CSR0 reads something other than 0 and all
    /// ones.
    fn wait_for_mac(&self) -> Result<(), Error> {
        for _ in 0..500 {
            match self.read(Space::Mac, regs::MAC_CSR0)? {
                0 | u32::MAX => thread::sleep(ms(5)),
                _ => return Ok(()),
            }
        }
        Err(Error::Timeout("the MAC to come up"))
    }

    /// mt76x2u_mcu_load_rom_patch, for the MT7612 (no ROM protection).
    fn load_rom_patch(&mut self, patch: &RomPatch, rev: u32) -> Result<(), Error> {
        let (patch_reg, patch_mask) = if rev >= regs::REV_E3 {
            (regs::MCU_CLOCK_CTL, 1 << 0)
        } else {
            (regs::MCU_COM_REG0, 1 << 1)
        };
        self.write(
            Space::Cfg,
            regs::USB_U3DMA_CFG,
            regs::USB_DMA_CFG_RX_BULK_EN
                | regs::USB_DMA_CFG_TX_BULK_EN
                | (0x20 & regs::USB_DMA_CFG_RX_BULK_AGG_TOUT),
        )?;
        self.fw_reset()?;
        self.fce_for_firmware()?;
        self.send_firmware(
            patch.body,
            mcu::ROM_PATCH_MAX_PAYLOAD,
            mcu::ROM_PATCH_OFFSET,
        )?;
        // mt76x2u_mcu_enable_patch and mt76x2u_mcu_reset_wmt: class requests.
        self.request(
            ControlType::Class,
            req::DEV_MODE,
            0x12,
            0,
            &[
                0x6f, 0xfc, 0x08, 0x01, 0x20, 0x04, 0x00, 0x00, 0x00, 0x09, 0x00,
            ],
        )?;
        self.request(
            ControlType::Class,
            req::DEV_MODE,
            0x12,
            0,
            &[0x6f, 0xfc, 0x05, 0x01, 0x07, 0x01, 0x00, 0x04],
        )?;
        thread::sleep(ms(20));
        if !self.poll(Space::Mac, patch_reg, patch_mask, patch_mask, ms(100))? {
            return Err(Error::Timeout("the ROM patch to apply"));
        }
        Ok(())
    }

    /// mt76x2u_mcu_load_firmware.
    fn load_firmware(&mut self, fw: &Firmware, rev: u32) -> Result<(), Error> {
        self.fw_reset()?;
        self.write(
            Space::Cfg,
            regs::USB_U3DMA_CFG,
            regs::USB_DMA_CFG_RX_BULK_EN
                | regs::USB_DMA_CFG_TX_BULK_EN
                | (0x20 & regs::USB_DMA_CFG_RX_BULK_AGG_TOUT),
        )?;
        self.fce_for_firmware()?;
        self.send_firmware(fw.ilm, mcu::FW_MAX_PAYLOAD, mcu::ILM_OFFSET)?;
        let dlm = mcu::DLM_OFFSET + if rev >= regs::REV_E3 { 0x800 } else { 0 };
        self.send_firmware(fw.dlm, mcu::FW_MAX_PAYLOAD, dlm)?;
        // mt76x2u_mcu_load_ivb
        self.request(ControlType::Vendor, req::DEV_MODE, 0x12, 0, &[])?;
        if !self.poll(Space::Mac, regs::MCU_COM_REG0, 1, 1, ms(100))? {
            return Err(Error::Timeout("the firmware to start"));
        }
        self.set(Space::Mac, regs::MCU_COM_REG0, 1 << 1)?;
        // enable FCE to send in-band cmd
        self.write(Space::Mac, regs::FCE_PSE_CTRL, 1)
    }

    /// mt76x02u_mcu_fw_reset: a vendor reset of the MCU.
    fn fw_reset(&self) -> Result<(), Error> {
        self.request(ControlType::Vendor, req::DEV_MODE, 0x1, 0, &[])?;
        thread::sleep(ms(10));
        Ok(())
    }

    /// The FCE set up to take in-band commands, as both firmware loads do.
    fn fce_for_firmware(&self) -> Result<(), Error> {
        self.write(Space::Mac, regs::FCE_PSE_CTRL, 0x1)?;
        self.write(Space::Mac, regs::TX_CPU_FROM_FCE_BASE_PTR, 0x40_0230)?;
        self.write(Space::Mac, regs::TX_CPU_FROM_FCE_MAX_COUNT, 0x1)?;
        self.write(Space::Mac, regs::FCE_PDMA_GLOBAL_CONF, 0x44)?;
        self.write(Space::Mac, regs::FCE_SKIP_FS, 0x3)
    }

    /// mt76x02u_mcu_fw_send_data: `data` to `offset` in the MCU's memory, in
    /// bulk transfers of at most `max_payload`.
    fn send_firmware(&mut self, data: &[u8], max_payload: usize, offset: u32) -> Result<(), Error> {
        for (i, piece) in data.chunks(max_payload - 8).enumerate() {
            let at = offset + (i * (max_payload - 8)) as u32;
            self.fce_write(regs::FCE_DMA_ADDR, at)?;
            self.fce_write(regs::FCE_DMA_LEN, mcu::chunk_dma_len(piece.len()))?;
            self.bulk_out(mcu::firmware_chunk(piece), ms(1000))?;
            let idx = self.read(Space::Mac, regs::TX_CPU_FROM_FCE_CPU_DESC_IDX)?;
            self.write(
                Space::Mac,
                regs::TX_CPU_FROM_FCE_CPU_DESC_IDX,
                idx.wrapping_add(1),
            )?;
            thread::sleep(ms(5));
        }
        Ok(())
    }

    /// mt76x2u_init_dma: one frame per bulk IN transfer.
    fn init_dma(&self) -> Result<(), Error> {
        let mut val = self.read(Space::Cfg, regs::USB_U3DMA_CFG)?;
        val |= regs::USB_DMA_CFG_RX_DROP_OR_PAD
            | regs::USB_DMA_CFG_RX_BULK_EN
            | regs::USB_DMA_CFG_TX_BULK_EN;
        val &= !regs::USB_DMA_CFG_RX_BULK_AGG_EN;
        self.write(Space::Cfg, regs::USB_U3DMA_CFG, val)
    }

    /// mt76u_single_wr with MT_VEND_WRITE_FCE: the value in two halves, in
    /// wValue, to `offset` and `offset + 2`.
    fn fce_write(&self, offset: u16, val: u32) -> Result<(), Error> {
        self.request(ControlType::Vendor, req::WRITE_FCE, val as u16, offset, &[])?;
        self.request(
            ControlType::Vendor,
            req::WRITE_FCE,
            (val >> 16) as u16,
            offset + 2,
            &[],
        )
    }

    fn bulk_out(&mut self, data: Vec<u8>, timeout: Duration) -> Result<(), Error> {
        Ok(self
            .cmd_out
            .transfer_blocking(Buffer::from(data), timeout)
            .status?)
    }

    /// A control request to the device, OUT.
    fn request(
        &self,
        control_type: ControlType,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), Error> {
        self.retry(|| {
            self.interface
                .control_out(
                    ControlOut {
                        control_type,
                        recipient: Recipient::Device,
                        request,
                        value,
                        index,
                        data,
                    },
                    TIMEOUT,
                )
                .wait()
        })
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
            thread::sleep(ms(5));
        }
        Err(last.expect("tried at least once").into())
    }
}

/// mt76_write_mac_initvals: "copied from MediaTek reference source", then the
/// protection settings (DEFAULT_PROT_CFG_CCK, _OFDM, _20 and _40 worked out).
const MAC_INITVALS: &[(u32, u32)] = &[
    (regs::PBF_SYS_CTRL, 0x0008_0c00),
    (regs::PBF_CFG, 0x1efe_bcff),
    (regs::FCE_PSE_CTRL, 0x0000_0001),
    (regs::MAC_SYS_CTRL, 0x0000_0000),
    (regs::MAX_LEN_CFG, 0x003e_3f00),
    (regs::AMPDU_MAX_LEN_20M1S, 0xaaa9_9887),
    (regs::AMPDU_MAX_LEN_20M2S, 0x0000_00aa),
    (regs::XIFS_TIME_CFG, 0x33a4_0d0a),
    (regs::BKOFF_SLOT_CFG, 0x0000_0209),
    (regs::TBTT_SYNC_CFG, 0x0042_2010),
    (regs::PWR_PIN_CFG, 0x0000_0000),
    (0x1238, 0x0017_00c8),
    (regs::TX_SW_CFG0, 0x0010_1001),
    (regs::TX_SW_CFG1, 0x0001_0000),
    (regs::TX_SW_CFG2, 0x0000_0000),
    (regs::TXOP_CTRL_CFG, 0x0400_583f),
    (regs::TX_RTS_CFG, 0x00ff_ff20),
    (regs::TX_TIMEOUT_CFG, 0x000a_2290),
    (regs::TX_RETRY_CFG, 0x47f0_1f0f),
    (regs::EXP_ACK_TIME, 0x002c_00dc),
    (regs::TX_PROT_CFG6, 0xe3f4_2004),
    (regs::TX_PROT_CFG7, 0xe3f4_2084),
    (regs::TX_PROT_CFG8, 0xe3f4_2104),
    (regs::PIFS_TX_CFG, 0x0006_0fff),
    (regs::RX_FILTR_CFG, 0x0001_5f97),
    (regs::LEGACY_BASIC_RATE, 0x0000_017f),
    (regs::HT_BASIC_RATE, 0x0000_4003),
    (regs::PN_PAD_MODE, 0x0000_0003),
    (regs::TXOP_HLDR_ET, 0x0000_0002),
    (0xa44, 0x0000_0000),
    (regs::HEADER_TRANS_CTRL_REG, 0x0000_0000),
    (regs::TSO_CTRL, 0x0000_0000),
    (regs::AUX_CLK_CFG, 0x0000_0000),
    (regs::DACCLK_EN_DLY_CFG, 0x0000_0000),
    (regs::TX_ALC_CFG_4, 0x0000_0000),
    (regs::TX_ALC_VGA3, 0x0000_0000),
    (regs::TX_PWR_CFG_0, 0x3a3a_3a3a),
    (regs::TX_PWR_CFG_1, 0x3a3a_3a3a),
    (regs::TX_PWR_CFG_2, 0x3a3a_3a3a),
    (regs::TX_PWR_CFG_3, 0x3a3a_3a3a),
    (regs::TX_PWR_CFG_4, 0x3a3a_3a3a),
    (regs::TX_PWR_CFG_7, 0x3a3a_3a3a),
    (regs::TX_PWR_CFG_8, 0x0000_003a),
    (regs::TX_PWR_CFG_9, 0x0000_003a),
    (regs::EFUSE_CTRL, 0x0000_d000),
    (regs::PAUSE_ENABLE_CONTROL1, 0x0000_000a),
    (regs::FCE_WLAN_FLOW_CONTROL1, 0x6040_1c18),
    (regs::WPDMA_DELAY_INT_CFG, 0x94ff_0000),
    (regs::TX_SW_CFG3, 0x0000_0004),
    (regs::HT_FBK_TO_LEGACY, 0x0000_1818),
    (regs::VHT_HT_FBK_CFG1, 0xedcb_a980),
    (regs::PROT_AUTO_TX_CFG, 0x0083_0083),
    (regs::HT_CTRL_CFG, 0x0000_01ff),
    (regs::TX_LINK_CFG, 0x0000_1020),
    // rate 0x3, NAV 1, TXOP allow 0x3f, RTS threshold
    (regs::CCK_PROT_CFG, 0x07f4_0003),
    // rate 0x2004, NAV 1, TXOP allow 0x3f, RTS threshold
    (regs::OFDM_PROT_CFG, 0x07f4_2004),
    // rate 0x2004, ctrl 1, NAV 1, TXOP allow 0x17
    (regs::MM20_PROT_CFG, 0x0175_2004),
    // rate 0x2084, ctrl 1, NAV 1, TXOP allow 0x3f
    (regs::MM40_PROT_CFG, 0x03f5_2084),
    (regs::GF20_PROT_CFG, 0x0175_2004),
    (regs::GF40_PROT_CFG, 0x03f5_2084),
];

/// A 16-bit EEPROM value (mt76x02_eeprom_get); 0xffff past its end.
fn ee16(eeprom: &[u8], offset: usize) -> u16 {
    match eeprom.get(offset..offset + 2) {
        Some(b) => u16::from_le_bytes([b[0], b[1]]),
        None => 0xffff,
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
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
