//! Milestone 1 (docs/native-windows.md): find an MT7612U stick, read its chip
//! ID and its MAC address. Needs the stick on WinUSB on Windows; on Linux,
//! root (or a udev rule) and the stick attached, and it takes the stick from
//! the kernel's mt76x2u.
//!
//!   cargo run -p mt7612u --example info

use std::process::ExitCode;

use mt7612u::{Mt7612u, Space, eeprom_mac, format_mac};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), mt7612u::Error> {
    let stick = Mt7612u::open()?;
    let info = stick.info();
    println!(
        "stick: {:04x}:{:04x} {} {}",
        info.vendor_id(),
        info.product_id(),
        info.manufacturer_string().unwrap_or("?"),
        info.product_string().unwrap_or("?")
    );
    println!("ASIC version: {:#010x}", stick.asic_version()?);
    println!("MAC_CSR0: {:#010x}", stick.read(Space::Mac, 0x1000)?);
    let eeprom = stick.eeprom()?;
    match eeprom_mac(&eeprom) {
        Some(mac) => println!("MAC address: {}", format_mac(&mac)),
        None => println!("MAC address: not in the EEPROM"),
    }
    println!("EEPROM, first 32 bytes: {:02x?}", &eeprom[..32]);
    Ok(())
}
