//! Milestone 2 (docs/native-windows.md): power the stick on, start its MCU
//! with the ROM patch and firmware, and check the MCU answers: a register
//! read through it must match the same register read directly.
//!
//!   cargo run -p mt7612u --example firmware -- DIR
//!
//! DIR holds mt7662_rom_patch.bin and mt7662.bin (linux-firmware's).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use mt7612u::{Mt7612u, Space, regs};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args().nth(1).ok_or("usage: firmware DIR")?);
    let rom_patch = std::fs::read(dir.join("mt7662_rom_patch.bin"))?;
    let firmware = std::fs::read(dir.join("mt7662.bin"))?;

    let mut stick = Mt7612u::open()?;
    println!("ASIC version: {:#010x}", stick.asic_version()?);

    let t = Instant::now();
    let started = stick.start_mcu(&rom_patch, &firmware)?;
    println!("ROM patch build: {}", started.rom_patch_build);
    println!(
        "Firmware {} build {:x} ({}): running after {:.2} s",
        started.firmware_version,
        started.firmware_build,
        started.firmware_build_time,
        t.elapsed().as_secs_f64()
    );
    println!(
        "MCU_COM_REG0: {:#010x}",
        stick.read(Space::Mac, regs::MCU_COM_REG0)?
    );

    let asked = [regs::ASIC_VERSION, regs::MAC_CSR0];
    let by_mcu = stick.mcu_read(regs::MCU_MEMMAP_WLAN, &asked)?;
    for (reg, mcu) in asked.iter().zip(&by_mcu) {
        let direct = stick.read(Space::Mac, *reg)?;
        let same = if *mcu == direct { "same" } else { "DIFFERENT" };
        println!("{reg:#06x}: {mcu:#010x} by the MCU, {direct:#010x} directly: {same}");
    }
    if by_mcu
        .iter()
        .zip(asked)
        .any(|(mcu, reg)| stick.read(Space::Mac, reg).ok() != Some(*mcu))
    {
        return Err("the MCU's reads don't match".into());
    }
    println!("The MCU answers.");
    Ok(())
}
