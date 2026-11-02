//! A transmit check (milestone 4 of docs/native-windows.md): start the
//! stick on channel 149 at 40 MHz, send broadcast vendor action frames that
//! no AWDL device parses (OUI 00:00:00), time how long the chip takes each
//! one, and read its own counters of frames sent (TX_STA_*, cleared on read).
//!
//!   RUST_LOG=mt7612u=debug cargo run -p mt7612u --example send -- FIRMWARE_DIR [COUNT]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mt7612u::channel::{Channel, Width};
use mt7612u::tx::Rate;
use mt7612u::{Mt7612u, Space, eeprom_mac};
use tracing_subscriber::EnvFilter;

const TX_STA_0: u32 = 0x170c; // failed (low half), beacons (high)
const TX_STA_1: u32 = 0x1710; // sent (low half), retried (high)
const TX_STA_2: u32 = 0x1714;
const MAC_STATUS: u32 = 0x1200;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().ok_or("usage: send FIRMWARE_DIR [COUNT]")?);
    let count: usize = args.next().map(|s| s.parse()).transpose()?.unwrap_or(200);
    let rom_patch = std::fs::read(dir.join("mt7662_rom_patch.bin"))?;
    let firmware = std::fs::read(dir.join("mt7662.bin"))?;

    let t = Instant::now();
    let mut stick = Mt7612u::open()?;
    let eeprom = stick.eeprom()?;
    let mac = eeprom_mac(&eeprom).ok_or("no MAC address in the EEPROM")?;
    stick.start_mcu(&rom_patch, &firmware)?;
    println!("MCU running after {:.2} s", t.elapsed().as_secs_f64());
    stick.init_mac(&eeprom)?;
    stick.set_channel(&eeprom, Channel::new(149, Width::Ht40Plus))?;
    stick.start_rx()?;
    println!("Radio up after {:.2} s", t.elapsed().as_secs_f64());
    for reg in [TX_STA_0, TX_STA_1, TX_STA_2] {
        stick.read(Space::Mac, reg)?;
    }

    // One at a time first: how long the chip takes one frame.
    let mut waits = Vec::new();
    for seq in 0..10u16 {
        let sent = Instant::now();
        stick
            .send(frame(&mac, seq), Rate::Legacy(24), true)
            .map_err(|e| format!("{e:?}"))?;
        let left = stick.flush_tx(Duration::from_secs(2));
        waits.push((sent.elapsed().as_micros(), left));
    }
    println!("One at a time (µs, left over): {waits:?}");

    // Then a burst, as the node sends: 40 ms apart, as fast as the queue takes.
    let start = Instant::now();
    let mut full = 0;
    for seq in 10..count as u16 {
        let mut f = frame(&mac, seq);
        loop {
            match stick.send(f, Rate::Legacy(24), true) {
                Ok(()) => break,
                Err(mt7612u::SendError::Full(back)) => {
                    full += 1;
                    f = back;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(format!("{e:?}").into()),
            }
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    let left = stick.flush_tx(Duration::from_secs(3));
    println!(
        "{} sent in {:.2} s, queue full {full} times, {left} still pending, {} USB errors",
        count - 10,
        start.elapsed().as_secs_f64(),
        stick.tx_errors()
    );

    let sta0 = stick.read(Space::Mac, TX_STA_0)?;
    let sta1 = stick.read(Space::Mac, TX_STA_1)?;
    let sta2 = stick.read(Space::Mac, TX_STA_2)?;
    println!(
        "Chip: sent {}, retried {}, failed {}, STA_2 {sta2:#010x}, MAC_STATUS {:#x}",
        sta1 & 0xffff,
        sta1 >> 16,
        sta0 & 0xffff,
        stick.read(Space::Mac, MAC_STATUS)?
    );
    Ok(())
}

/// A broadcast vendor action frame no one parses: category 127, OUI 00:00:00.
fn frame(mac: &[u8; 6], seq: u16) -> Vec<u8> {
    let mut f = vec![0xd0, 0x00, 0x00, 0x00];
    f.extend_from_slice(&[0xff; 6]);
    f.extend_from_slice(mac);
    f.extend_from_slice(mac);
    f.extend_from_slice(&(seq << 4).to_le_bytes());
    f.extend_from_slice(&[127, 0x00, 0x00, 0x00]);
    f.extend_from_slice(&[0u8; 60]);
    f
}
