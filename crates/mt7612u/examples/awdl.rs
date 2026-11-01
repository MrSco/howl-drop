//! Milestone 4 (docs/native-windows.md): an owly-awdl node on the stick, on
//! channel 149 at 40 MHz, with no network interface yet. Peers it syncs with
//! are printed as they come and go; an iPhone nearby with its share sheet
//! open should be one.
//!
//!   RUST_LOG=info cargo run -p mt7612u --example awdl -- FIRMWARE_DIR [SECONDS]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mt7612u::awdl::{NoNetif, Radio};
use mt7612u::channel::{Channel, Width};
use mt7612u::{Mt7612u, eeprom_mac, format_mac};
use owly_awdl::{MacAddr, Presence, RadioConstraints};
use tracing_subscriber::EnvFilter;

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
    let dir = PathBuf::from(args.next().ok_or("usage: awdl FIRMWARE_DIR [SECONDS]")?);
    let seconds: u64 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(60);
    let rom_patch = std::fs::read(dir.join("mt7662_rom_patch.bin"))?;
    let firmware = std::fs::read(dir.join("mt7662.bin"))?;

    let t = Instant::now();
    let mut stick = Mt7612u::open()?;
    let eeprom = stick.eeprom()?;
    let mac = eeprom_mac(&eeprom).ok_or("no MAC address in the EEPROM")?;
    stick.start_mcu(&rom_patch, &firmware)?;
    stick.init_mac(&eeprom)?;
    stick.set_channel(&eeprom, Channel::new(149, Width::Ht40Plus))?;
    stick.start_rx()?;
    println!(
        "Radio up on 149 HT40+ after {:.2} s, AWDL address {}",
        t.elapsed().as_secs_f64(),
        format_mac(&mac)
    );

    let radio = RadioConstraints {
        channel: owly_awdl::Channel(149),
        presence: Presence::Fixed,
    };
    let cfg = owly_awdl::Config::new(MacAddr(mac), "howldrop-native", radio);
    let (peers_tx, peers) = tokio::sync::watch::channel(Vec::new());
    let node = owly_awdl::driver::spawn(cfg, move || Ok((Radio::new(stick), NoNetif)), peers_tx)?;

    let mut known: Vec<MacAddr> = Vec::new();
    let until = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < until {
        if let Some(why) = node.failed().borrow().clone() {
            return Err(format!("the node stopped: {why}").into());
        }
        let now = peers.borrow().clone();
        for p in now.iter().filter(|p| !known.contains(&p.mac)) {
            let at = t.elapsed().as_secs_f64();
            println!(
                "{at:6.1} s  peer up: {} {:?}, RSSI {:?}",
                format_mac(&p.mac.0),
                p.name,
                p.rssi
            );
        }
        for gone in known.iter().filter(|m| !now.iter().any(|p| p.mac == **m)) {
            println!(
                "{:6.1} s  peer gone: {}",
                t.elapsed().as_secs_f64(),
                format_mac(&gone.0)
            );
        }
        known = now.iter().map(|p| p.mac).collect();
        std::thread::sleep(Duration::from_millis(500));
    }
    println!("{} peer(s) at the end", known.len());
    node.stop();
    Ok(())
}
