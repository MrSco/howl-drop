//! howldrop-agent: HowlDrop's Linux side in one process, on Owly's crates.
//! The Home Assistant add-on (run.sh) and the Windows app's WSL part
//! (wsl-run.sh) both run it:
//!
//! - the radio ([`radio`]): owly-radio's injection backend on the best Wi-Fi
//!   card here, in the mode its driver needs, given back on the way out;
//! - receiving ([`receive`]): owly-airdrop's receiver on `awdl0`, with who
//!   may send (everyone, ask the PC, contacts only), into a staging folder
//!   deliver.py takes from;
//! - sending ([`send`]): what's put in the outbox folder, AirDropped to the
//!   iPhone nearby, and the nearby devices when asked.
//!
//! It prints log lines, and with `--events` the `@howldrop` events
//! howldrop.exe reads. SIGTERM or SIGINT takes the radio down.

mod bluez;
mod contacts;
mod events;
mod radio;
mod receive;
mod send;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use owly_airdrop::driver::tls::Identity;
use owly_radio::radio::LinkState;
use tokio::signal::unix::{SignalKind, signal};

use crate::events::{emit, say};
use crate::receive::Policy;

#[derive(Parser)]
#[command(name = "howldrop-agent", version, about)]
struct Args {
    /// The name the iPhone's share sheet shows.
    #[arg(long, default_value = "HowlDrop")]
    name: String,
    /// The Mac model reported to senders.
    #[arg(long, default_value = "MacBookPro18,3")]
    model: String,
    /// Where received files go, for deliver.py.
    #[arg(long)]
    staging: PathBuf,
    /// Where the receiver's TLS identity is kept.
    #[arg(long, default_value = "/data/owly")]
    identity: PathBuf,
    /// Who may send: everyone, ask (the PC's user, through --ask-dir) or
    /// contacts.
    #[arg(long, default_value = "everyone")]
    receive: Policy,
    /// Contacts: phone numbers (with their country code) or emails,
    /// separated by commas, semicolons or new lines.
    #[arg(long, default_value = "")]
    senders: String,
    /// Where the PC answers an ask with ID.accept or ID.decline.
    #[arg(long)]
    ask_dir: Option<PathBuf>,
    /// Seconds the PC's user has to answer.
    #[arg(long, default_value_t = 60)]
    ask_timeout: u64,
    /// AirDrop what's put here to the iPhone; no sending without it.
    #[arg(long)]
    outbox: Option<PathBuf>,
    /// Delete each file once tried, instead of moving it to sent/ or failed/.
    #[arg(long)]
    discard: bool,
    /// The iPhone's name, or part of it; else the one nearby.
    #[arg(long)]
    to: Option<String>,
    /// Seconds to keep trying one file.
    #[arg(long, default_value_t = 180)]
    window: u64,
    /// The latest sends as JSON, for the panel.
    #[arg(long)]
    send_status: Option<PathBuf>,
    /// The latest scan as JSON (and a .txt twin, for the relay).
    #[arg(long)]
    nearby: Option<PathBuf>,
    /// A file whose appearance asks for a scan.
    #[arg(long)]
    scan_request: Option<PathBuf>,
    /// Wake the iPhone with this machine's Bluetooth (BlueZ) when sending.
    #[arg(long)]
    ble: bool,
    /// Print @howldrop events for howldrop.exe.
    #[arg(long)]
    events: bool,
    /// The Wi-Fi interface to use; else the best one here.
    #[arg(long)]
    interface: Option<String>,
    /// Run on a card the quirks table doesn't mark tested.
    #[arg(long)]
    allow_untested: bool,
    /// AirDrop channels to try, in order: "149 44 6"; empty or auto for the
    /// card's.
    #[arg(long, default_value = "")]
    channels: String,
    /// Accept the phone's Block Ack sessions: on, off or auto.
    #[arg(long, default_value = "auto")]
    aggregation: String,
    /// The regulatory domain, two capitals; else US where none is set.
    #[arg(long)]
    country: Option<String>,
    /// The data frame rate: 6–54 (Mbit/s), or mcs0–mcs15.
    #[arg(long)]
    tx_rate: Option<String>,
    /// The radio's recovery journal.
    #[arg(long, default_value = owly_radio::rundir::RUN_DIR)]
    run_dir: String,
    #[arg(long)]
    verbose: bool,
}

/// Restarts of a link that went away by itself, before giving up.
const RESTARTS: u32 = 3;
/// A link that held this long has its restarts back.
const STEADY: Duration = Duration::from_secs(600);

fn main() -> ExitCode {
    let args = Args::parse();
    let filter = if args.verbose {
        "info,owly_radio=debug"
    } else {
        "warn,owly_radio=info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .with_writer(std::io::stdout)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .init();
    // rustls needs its crypto provider chosen.
    let _ = rustls::crypto::ring::default_provider().install_default();
    if args.events {
        events::enable();
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("a tokio runtime");
    match rt.block_on(run(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            say(e);
            ExitCode::from(1)
        }
    }
}

async fn run(args: Args) -> Result<(), String> {
    let radio_opts = radio::Options {
        station: args
            .interface
            .clone()
            .filter(|s| !s.is_empty() && s != "auto"),
        allow_untested: args.allow_untested,
        channels: radio::channels(&args.channels)?,
        aggregation: radio::aggregation(&args.aggregation)?,
        country: args.country.clone().filter(|c| !c.is_empty()),
        tx_rate: args.tx_rate.clone().filter(|r| !r.is_empty()),
        run_dir: args.run_dir.clone(),
    };
    let (contacts, notes) = contacts::Contacts::new(&contacts::entries(&args.senders))
        .map_err(|e| format!("contacts: {e}"))?;
    for n in notes {
        say(format!("Senders: {n}"));
    }
    let mut policy = args.receive;
    if policy == Policy::Contacts && contacts.is_empty() {
        say("Contacts only, but no contacts are set: everyone can send until some are.");
        policy = Policy::Everyone;
    }
    if policy == Policy::Ask && args.ask_dir.is_none() {
        return Err("--receive ask needs --ask-dir".into());
    }
    if let Some(dir) = &args.ask_dir {
        let _ = std::fs::create_dir_all(dir);
        // Answers to a run before this one.
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.ends_with(".accept") || n.ends_with(".decline") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let contacts = Arc::new(contacts);
    let identity = Identity::load_or_generate(&args.identity).map_err(|e| {
        format!(
            "the receiver's identity in {}: {e}",
            args.identity.display()
        )
    })?;

    let mut radio = radio::Radio::new(&radio_opts)?;
    radio.report();
    if let Some(why) = radio.unusable() {
        return Err(why);
    }
    radio.recover().await;
    let station = radio.station();

    let mut term = signal(SignalKind::terminate()).map_err(|e| e.to_string())?;
    let mut int = signal(SignalKind::interrupt()).map_err(|e| e.to_string())?;
    let mut restarts = 0;
    let result = loop {
        say(format!("Starting the AirDrop window on {station}."));
        let up = tokio::select! {
            up = radio.up() => up,
            _ = term.recv() => break Ok(()),
            _ = int.recv() => break Ok(()),
        };
        let mut link = match up {
            Ok(link) => link,
            Err(e) => {
                break Err(format!(
                    "The AirDrop window did not start on {station}: {e}"
                ));
            }
        };
        let started = Instant::now();
        let receiving = receive::start(
            &link,
            receive::Options {
                name: args.name.clone(),
                model: args.model.clone(),
                staging: args.staging.clone(),
                identity: identity.clone(),
                policy,
                ask_dir: args.ask_dir.clone(),
                ask_timeout: Duration::from_secs(args.ask_timeout),
                contacts: contacts.clone(),
                verbose: args.verbose,
            },
        )
        .await;
        let receiving = match receiving {
            Ok(r) => r,
            Err(e) => {
                radio.down().await;
                break Err(format!("The AirDrop receiver didn't start: {e}"));
            }
        };
        let sending = args.outbox.clone().map(|outbox| {
            tokio::spawn(send::run(
                send::Air {
                    index: link.index,
                    peers: link.peers.clone(),
                },
                send::Options {
                    outbox,
                    discard: args.discard,
                    name: args.name.clone(),
                    to: args.to.clone().filter(|t| !t.trim().is_empty()),
                    window: Duration::from_secs(args.window),
                    status: args.send_status.clone(),
                    nearby: args.nearby.clone(),
                    scan_request: args.scan_request.clone(),
                    ble: args.ble,
                },
            ))
        });
        say(format!(
            "Window is up on {station} ({}%{}). Stopping gives the card back.",
            link.addr, link.name
        ));
        say(format!("The iPhone share sheet shows: {}", args.name));
        emit(&["up", &station]);

        let lost = loop {
            tokio::select! {
                _ = term.recv() => break None,
                _ = int.recv() => break None,
                changed = link.state.changed() => {
                    if changed.is_err() {
                        break Some("the radio stopped".to_string());
                    }
                    if let LinkState::Lost(why) = &*link.state.borrow() {
                        break Some(why.clone());
                    }
                }
            }
        };
        if let Some(s) = sending {
            s.abort();
        }
        drop(receiving);
        radio.down().await;
        let Some(why) = lost else { break Ok(()) };
        if started.elapsed() >= STEADY {
            restarts = 0;
        }
        restarts += 1;
        if restarts > RESTARTS {
            break Err(format!("The AirDrop window keeps stopping: {why}"));
        }
        say(format!(
            "The AirDrop window stopped ({why}); starting it again."
        ));
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    if restarts > 0 || result.is_ok() {
        say(format!("{station} is given back."));
    }
    result
}
