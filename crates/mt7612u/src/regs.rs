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
