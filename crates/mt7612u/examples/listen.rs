//! Milestone 3 (docs/native-windows.md): start the stick, tune it to channel
//! 149 at 40 MHz (HT40+), and listen: every frame counted, AWDL action frames
//! by sender. An iPhone nearby with AirDrop open sends them.
//!
//!   cargo run -p mt7612u --example listen -- FIRMWARE_DIR [SECONDS]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mt7612u::channel::{Channel, Width};
use mt7612u::{Mt7612u, format_mac, rx};

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
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().ok_or("usage: listen FIRMWARE_DIR [SECONDS]")?);
    let seconds: u64 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(20);
    let rom_patch = std::fs::read(dir.join("mt7662_rom_patch.bin"))?;
    let firmware = std::fs::read(dir.join("mt7662.bin"))?;

    let t = Instant::now();
    let mut stick = Mt7612u::open()?;
    let eeprom = stick.eeprom()?;
    stick.start_mcu(&rom_patch, &firmware)?;
    println!("MCU running after {:.2} s", t.elapsed().as_secs_f64());
    stick.init_mac(&eeprom)?;
    println!("MAC set up after {:.2} s", t.elapsed().as_secs_f64());
    let ch = Channel::new(149, Width::Ht40Plus);
    stick.set_channel(&eeprom, ch)?;
    println!("Tuned to {ch:?} after {:.2} s", t.elapsed().as_secs_f64());
    stick.start_rx()?;
    println!("Listening for {seconds} s...");

    let mut total = 0usize;
    let mut bad_crc = 0usize;
    let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut awdl: BTreeMap<[u8; 6], (usize, i32)> = BTreeMap::new();
    let mut shown = 0;
    let until = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < until {
        let Some(f) = stick.next_frame(Duration::from_millis(200))? else {
            continue;
        };
        total += 1;
        if f.crc_error() {
            bad_crc += 1;
            continue;
        }
        *kinds.entry(kind(&f.frame)).or_default() += 1;
        if rx::is_awdl(&f.frame) {
            let from = rx::transmitter(&f.frame).unwrap_or_default();
            let e = awdl.entry(from).or_default();
            e.0 += 1;
            e.1 = i32::from(f.rssi[0] as i8);
            if shown < 3 {
                shown += 1;
                let n = f.frame.len().min(48);
                println!(
                    "AWDL from {} ({} bytes, rate {:#06x}): {:02x?}...",
                    format_mac(&from),
                    f.frame.len(),
                    f.rate,
                    &f.frame[..n]
                );
            }
        }
    }

    println!("{total} frames in {seconds} s ({bad_crc} with a bad CRC)");
    for (k, n) in &kinds {
        println!("  {k}: {n}");
    }
    if awdl.is_empty() {
        println!("No AWDL frames: is an iPhone nearby with AirDrop open?");
    }
    for (from, (n, rssi)) in &awdl {
        println!(
            "  AWDL from {}: {n} frames, last RSSI {rssi}",
            format_mac(from)
        );
    }
    Ok(())
}

fn kind(frame: &[u8]) -> &'static str {
    match (frame[0] >> 2) & 0x3 {
        0 => match frame[0] >> 4 {
            0x8 => "beacon",
            0x4 => "probe request",
            0x5 => "probe response",
            0xd => "action",
            _ => "other management",
        },
        1 => "control",
        2 => "data",
        _ => "extension",
    }
}
