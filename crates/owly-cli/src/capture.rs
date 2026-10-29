//! `owly capture`: records an AWDL session for the capture corpus
//! (`tests/captures/`), on the radio's monitor interface while a session is
//! up (`owly radio up` in another terminal).
//!
//! The corpus is committed, so nothing personal is written
//! ([`owly_radio::capture`]): only AWDL's frames to and from the phone and
//! this computer, with names, packet contents and unknown TLVs blanked,
//! every MAC address replaced by a stand-in, a radiotap header of just the
//! channel and signal, and time counted from the first frame. Frames are
//! held in memory until then. The stand-ins are told on the terminal, and
//! go nowhere else.

use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use owly_awdl::MacAddr;
use owly_radio::capture::{Filter, Pseudonyms, addresses};
use owly_radio::{native, pcap, radiotap};

#[derive(clap::Args)]
pub struct Args {
    /// What the capture shows, for its name: discovery, receive, send,
    /// restart, padding, amsdu, addba, ...
    what: String,
    /// The phone's iOS version, such as 26.1.
    #[arg(long)]
    ios: String,
    /// The phone's AWDL address (from `owly radio peers`). Repeat for each
    /// device to keep; without any, every AWDL device's frames are kept.
    #[arg(long = "peer", value_name = "MAC")]
    peers: Vec<String>,
    /// The monitor interface to capture on.
    #[arg(long, default_value = "owlymon0")]
    monitor: String,
    /// Stop after this many seconds, rather than at Ctrl-C.
    #[arg(long)]
    seconds: Option<u64>,
    /// Where to write the capture.
    #[arg(long, default_value = "tests/captures")]
    out: PathBuf,
}

/// A kept frame, its addresses still real.
struct Kept {
    micros: u64,
    freq: Option<u16>,
    rssi: Option<i8>,
    frame: Vec<u8>,
}

fn mac(s: &str) -> Result<MacAddr, String> {
    s.trim().parse().map_err(|e| format!("{s:?}: {e}"))
}

pub async fn run(args: Args) -> Result<ExitCode, String> {
    let mut keep = Vec::new();
    for p in &args.peers {
        keep.push(mac(p)?);
    }
    let local = std::fs::read_to_string("/sys/class/net/awdl0/address")
        .ok()
        .and_then(|a| mac(&a).ok());
    if keep.is_empty() {
        eprintln!(
            "owly: no --peer given, so every AWDL device's frames are kept, bystanders' too. Give the phone's address to leave them out."
        );
    } else if let Some(l) = local {
        keep.push(l);
    }
    let filter = Filter { keep };
    let index = nix::net::if_::if_nametoindex(args.monitor.as_str())
        .map_err(|e| format!("{}: {e}. Is a session up (`owly radio up`)?", args.monitor))?;
    let socket = native::packet_socket(index, true).map_err(|e| {
        format!(
            "capturing on {}: {e} (it takes root, or CAP_NET_RAW)",
            args.monitor
        )
    })?;
    let driver = std::fs::read_link(format!("/sys/class/net/{}/device/driver", args.monitor))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".into());

    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let capture = tokio::task::spawn_blocking(move || -> Result<(u64, Vec<Kept>), String> {
        let start = Instant::now();
        let mut kept = Vec::new();
        let mut seen = 0u64;
        let mut buf = vec![0u8; 16 * 1024];
        let mut report = Instant::now();
        while !flag.load(Ordering::Relaxed) {
            match nix::unistd::read(&socket, &mut buf) {
                Ok(n) => {
                    seen += 1;
                    let Some(rx) = radiotap::parse(&buf[..n]) else {
                        continue;
                    };
                    if let Some(frame) = filter.apply(rx.frame) {
                        kept.push(Kept {
                            micros: start.elapsed().as_micros() as u64,
                            freq: rx.freq,
                            rssi: rx.rssi,
                            frame,
                        });
                    }
                }
                Err(nix::errno::Errno::EAGAIN) => std::thread::sleep(Duration::from_millis(2)),
                Err(e) => return Err(format!("capturing: {e}")),
            }
            if report.elapsed() > Duration::from_secs(5) {
                report = Instant::now();
                eprintln!("{} AWDL frames kept of {seen} heard", kept.len());
            }
        }
        Ok((seen, kept))
    });
    eprintln!(
        "capturing on {} until {}...",
        args.monitor,
        args.seconds
            .map_or_else(|| "Ctrl-C".into(), |s| format!("{s} s have passed"))
    );
    match args.seconds {
        Some(s) => tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(s)) => {}
            _ = tokio::signal::ctrl_c() => {}
        },
        None => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
    stop.store(true, Ordering::Relaxed);
    let (seen, mut kept) = capture.await.map_err(|e| e.to_string())??;
    if kept.is_empty() {
        println!("no AWDL frames in {seen} heard; nothing written");
        return Ok(ExitCode::from(1));
    }

    // Stand-ins for everything, the peers given too, whether heard or not.
    let names = Pseudonyms::new(
        kept.iter()
            .flat_map(|k| addresses(&k.frame))
            .chain(filter_keep(&args.peers))
            .chain(local),
    );
    let channel = kept
        .iter()
        .find_map(|k| k.freq)
        .and_then(|f| owly_awdl::Channel::from_freq_mhz(f.into()))
        .map(|c| c.0);
    std::fs::create_dir_all(&args.out).map_err(|e| format!("{}: {e}", args.out.display()))?;
    let base = format!(
        "{}-ios{}-{}-ch{}",
        args.what,
        args.ios,
        driver,
        channel.map_or_else(|| "unknown".into(), |c| c.to_string()),
    );
    let name = free_name(&args.out, &base);
    let pcap_path = args.out.join(format!("{name}.pcap"));
    let file =
        std::fs::File::create(&pcap_path).map_err(|e| format!("{}: {e}", pcap_path.display()))?;
    let mut w = pcap::Writer::new(BufWriter::new(file)).map_err(|e| e.to_string())?;
    for k in &mut kept {
        names.apply(&mut k.frame);
        let mut record = radiotap::rx_header(k.freq, k.rssi);
        record.extend_from_slice(&k.frame);
        w.write(k.micros, &record).map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())?;

    let fake = |m: MacAddr| names.get(m).map_or_else(String::new, |f| f.to_string());
    let peers: Vec<String> = filter_keep(&args.peers)
        .map(|m| format!("\"{}\"", fake(m)))
        .collect();
    let expect = format!(
        "# What replaying {name}.pcap must find (crates/owly-radio/tests/replay.rs).\n\
         # `peers` are the --peer addresses' stand-ins; add `master = \"<stand-in>\"`\n\
         # once you know who it is.\n\
         channel = {}\n{}peers = [{}]\nnotes = {:?}\n",
        channel.unwrap_or(0),
        local.map_or_else(String::new, |l| format!("local = \"{}\"\n", fake(l))),
        peers.join(", "),
        format!("{} on iOS {}, {driver}", args.what, args.ios),
    );
    let toml_path = args.out.join(format!("{name}.toml"));
    std::fs::write(&toml_path, expect).map_err(|e| e.to_string())?;

    println!("Stand-ins (shown here only, written nowhere):");
    for (real, f) in names.iter() {
        let who = if Some(real) == local {
            " (this computer)"
        } else if filter_keep(&args.peers).any(|p| p == real) {
            " (--peer)"
        } else {
            ""
        };
        println!("  {f}  {real}{who}");
    }
    println!(
        "{} of {seen} frames in {}\nexpectations in {}",
        kept.len(),
        pcap_path.display(),
        toml_path.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn filter_keep(peers: &[String]) -> impl Iterator<Item = MacAddr> + '_ {
    peers.iter().filter_map(|p| mac(p).ok())
}

/// `base-1`, or the first `base-N` not taken.
fn free_name(dir: &Path, base: &str) -> String {
    (1..)
        .map(|n| format!("{base}-{n}"))
        .find(|n| !dir.join(format!("{n}.pcap")).exists())
        .expect("a free name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_numbered() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            free_name(dir.path(), "receive-ios26.1-x-ch6"),
            "receive-ios26.1-x-ch6-1"
        );
        std::fs::write(dir.path().join("receive-ios26.1-x-ch6-1.pcap"), "").unwrap();
        assert_eq!(
            free_name(dir.path(), "receive-ios26.1-x-ch6"),
            "receive-ios26.1-x-ch6-2"
        );
    }
}
