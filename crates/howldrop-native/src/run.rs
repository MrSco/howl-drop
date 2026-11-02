//! The radio, the node, the link and the receiver, until Ctrl+C or the
//! seconds asked for.

use std::net::SocketAddrV6;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mt7612u::Mt7612u;
use mt7612u::awdl::Radio;
use mt7612u::channel::{Channel, Width};
use owly_airdrop::driver::mdns::Mdns;
use owly_airdrop::driver::receiver::{Options as ReceiverOptions, Receiver, ReceiverEvent};
use owly_airdrop::driver::tls::Identity;
use owly_airdrop::proto::mdns::{self, RECEIVER_FLAGS, Service};
use owly_airdrop::proto::messages::Offer;
use owly_airdrop::proto::receiver::Config;
use owly_awdl::{MacAddr, Presence, RadioConstraints, TxRate};
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

use crate::link::Link;

/// AirDrop's port, as the agent serves it.
const PORT: u16 = 8771;
/// The channel the stick stays on (Presence::Fixed), at 40 MHz.
const CHANNEL: u8 = 149;

pub fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,owly_airdrop=info".into()),
        )
        .init();
    // rustls needs its crypto provider chosen.
    let _ = rustls::crypto::ring::default_provider().install_default();
    fine_timers();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    match rt.block_on(run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: howldrop-native FIRMWARE_DIR STAGING_DIR [NAME] [SECONDS]";
    let firmware = PathBuf::from(args.next().ok_or(usage)?);
    let staging = PathBuf::from(args.next().ok_or(usage)?);
    let name = args.next().unwrap_or_else(|| "HowlDrop Native".into());
    let seconds: Option<u64> = args
        .next()
        .map(|s| s.parse())
        .transpose()
        .map_err(|_| usage)?;
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
        .unwrap_or_default();
    std::fs::create_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;

    // The stick, register by register: blocking.
    let t = Instant::now();
    let (stick, mac) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let read = |f: &str| std::fs::read(firmware.join(f)).map_err(|e| format!("{f}: {e}"));
        let (rom_patch, fw) = (read("mt7662_rom_patch.bin")?, read("mt7662.bin")?);
        let err = |e: mt7612u::Error| e.to_string();
        let mut stick = Mt7612u::open().map_err(err)?;
        let eeprom = stick.eeprom().map_err(err)?;
        let mac = mt7612u::eeprom_mac(&eeprom).ok_or("no MAC address in the EEPROM")?;
        stick.start_mcu(&rom_patch, &fw).map_err(err)?;
        stick.init_mac(&eeprom).map_err(err)?;
        stick
            .set_channel(&eeprom, Channel::new(CHANNEL, Width::Ht40Plus))
            .map_err(err)?;
        stick.start_rx().map_err(err)?;
        Ok((stick, MacAddr(mac)))
    })
    .await
    .map_err(|e| e.to_string())??;
    println!(
        "Radio up on {CHANNEL} HT40+ after {:.2} s, AWDL address {mac}",
        t.elapsed().as_secs_f64()
    );

    let mut link = Link::create(&exe_dir.join("wintun.dll"), mac)?;
    let netif = link.netif().ok_or("the link's end is taken")?;
    println!(
        "Link up: Wintun adapter, interface {}, {}",
        link.index, link.addr
    );

    let mut cfg = owly_awdl::Config::new(
        mac,
        &name,
        RadioConstraints {
            channel: owly_awdl::Channel(CHANNEL),
            presence: Presence::Fixed,
        },
    );
    cfg.accept_addba = true;
    cfg.rate = TxRate::Legacy(72);
    let (peers_tx, mut peers) = watch::channel(Vec::new());
    let node = owly_awdl::driver::spawn(cfg, move || Ok((Radio::new(stick), netif)), peers_tx)
        .map_err(|e| format!("AWDL: {e}"))?;

    let identity_dir = staging.join(".identity");
    let identity = Identity::load_or_generate(&identity_dir)
        .map_err(|e| format!("the receiver's identity: {e}"))?;
    let mut options = ReceiverOptions::new(Config::new(name.clone()), staging.clone());
    options.progress = true;
    let addr = SocketAddrV6::new(link.addr, PORT, 0, link.index);
    let (receiver, mut events) = Receiver::bind(addr, &identity, options)
        .await
        .map_err(|e| format!("the receiver on {addr}: {e}"))?;
    tokio::spawn(receiver.run());
    let id = service_id(&identity_dir);
    let _registration = Mdns::new(link.index)
        .with_burst(Some(mdns::Burst {
            count: 64,
            interval: Duration::from_micros(16_400),
            pause: Duration::from_secs(3),
        }))
        .register(Service {
            instance: id.clone(),
            host: format!("howldrop-{id}"),
            port: PORT,
            addr: link.addr,
            flags: RECEIVER_FLAGS,
        })
        .await
        .map_err(|e| format!("mDNS on interface {}: {e}", link.index))?;
    println!(
        "On the share sheet as {name:?}; files go to {}",
        staging.display()
    );

    let until = seconds.map(|s| tokio::time::Instant::now() + Duration::from_secs(s));
    let mut asked: Option<Instant> = None;
    loop {
        let stop = async {
            match until {
                Some(u) => tokio::time::sleep_until(u).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            () = stop => break,
            Ok(()) = peers.changed() => {
                let list: Vec<String> = peers.borrow().iter().map(|p| p.mac.to_string()).collect();
                println!("peers: {list:?}");
            }
            Some(event) = events.recv() => match event {
                ReceiverEvent::Discovered { peer, .. } => println!("seen by {peer}"),
                ReceiverEvent::Request(r) => {
                    let what = match &r.ask.offer {
                        Offer::Files(files) => files
                            .iter()
                            .map(|f| format!("{} ({} bytes)", f.name, f.size.unwrap_or(0)))
                            .collect::<Vec<_>>()
                            .join(", "),
                        Offer::Urls(urls) => urls.join(", "),
                    };
                    println!("ask from {:?}: {what}; accepted", r.ask.sender_computer_name);
                    asked = Some(Instant::now());
                    r.accept();
                }
                ReceiverEvent::Received { paths, .. } => {
                    let took = asked.take().map(|a| a.elapsed().as_secs_f64()).unwrap_or(0.0);
                    for p in paths {
                        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                        println!("stored {} ({size} bytes, {took:.1} s after the ask)", p.display());
                    }
                }
                ReceiverEvent::Failed { reason, .. } => println!("failed: {reason}"),
                _ => {}
            },
        }
    }
    node.stop();
    link.close();
    println!("Stopped; the stick and the adapter are let go.");
    Ok(())
}

/// Timers to the millisecond for this process. Windows wakes them on its
/// 15.6 ms tick otherwise, most of an AWDL slot (16 TU) late, and the node's
/// frames, the peer's TCP ACKs among them, with them. Windows 11 also lets
/// a program without a window in sight fall back to the coarse tick unless
/// it asks not to.
#[allow(unsafe_code)]
fn fine_timers() {
    use windows_sys::Win32::Media::timeBeginPeriod;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
        ProcessPowerThrottling, SetProcessInformation,
    };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: 0,
    };
    // SAFETY: plain Win32 calls on this process; `state` outlives the call
    // and its size is given. The period lasts until the process ends.
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&raw const state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
        timeBeginPeriod(1);
    }
}

/// A 12-hex-digit service name, as Apple receivers' look, kept in `dir`: a
/// share sheet still showing the last run's entry reaches this one.
fn service_id(dir: &Path) -> String {
    let path = dir.join("service-id");
    if let Ok(id) = std::fs::read_to_string(&path)
        && id.len() == 12
        && id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return id;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let id = format!(
        "{:012x}",
        (nanos ^ u128::from(std::process::id()) << 20) & 0xffff_ffff_ffff
    );
    let _ = std::fs::write(&path, &id);
    id
}
