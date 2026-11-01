//! A user-space driver for the MediaTek MT7612U (Alfa AWUS036ACM, Netgear
//! A6210 and others): enough of Linux's mt76x2u (openwrt/mt76) to run the chip
//! as a monitor-and-inject radio for owly-awdl, over nusb: WinUSB on Windows,
//! usbfs on Linux. docs/native-windows.md has the plan.
//!
//! So far: finding a stick, reading its registers and EEPROM, powering it on
//! and starting its MCU with the ROM patch and firmware (mt76x2u_init_hardware
//! up to mt76x2u_mcu_init), and talking to the MCU.

pub mod mcu;
pub mod regs;

use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

use nusb::descriptors::TransferType;
use nusb::transfer::{
    Buffer, Bulk, ControlIn, ControlOut, ControlType, Direction, In, Out, Recipient, TransferError,
};
use nusb::{DeviceInfo, Endpoint, Interface, MaybeFuture};

use mcu::{Firmware, FirmwareError, RomPatch};

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
    /// The last MCU command's sequence number, 1 to 15.
    seq: u8,
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
        if ins.len() < 2 || outs.is_empty() {
            return Err(Error::Endpoints);
        }
        let cmd_out = interface.endpoint::<Bulk, Out>(outs[0])?;
        let resp_in = interface.endpoint::<Bulk, In>(ins[1])?;
        Ok(Mt7612u {
            info,
            interface,
            cmd_out,
            resp_in,
            seq: 0,
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

        self.reset_wlan()?;
        self.power_on()?;
        self.wait_for_mac()?;
        self.load_rom_patch(&rom_patch, rev)?;
        self.load_firmware(&firmware, rev)?;
        let busy = regs::WPDMA_GLO_CFG_TX_DMA_BUSY | regs::WPDMA_GLO_CFG_RX_DMA_BUSY;
        if !self.poll(Space::Mac, regs::WPDMA_GLO_CFG, busy, 0, ms(100))? {
            return Err(Error::Timeout("the DMA to go idle after the firmware"));
        }
        self.wait_for_mac()?;
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
