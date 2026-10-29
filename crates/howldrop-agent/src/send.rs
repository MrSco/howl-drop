//! Sending: AirDrops what's put in the outbox folder to a nearby iPhone,
//! as outbox.py did with omdrop-awdl's send-to-peer.
//!
//! A file is taken once its size has held for two looks a second apart,
//! skipping dotfiles, `*.partial` and `NAME.howldrop-to`, the sidecar that
//! names the device it's for (the panel, the relay and howldrop.exe write
//! it before the file). Without one, `--to`; without that, the only device
//! that answers, or of several the one whose name says iPhone. Names match
//! in part and either way round of a curly apostrophe ("Rocco’s iPhone").
//! Afterwards the file goes to `sent/` or `failed/` beside it, or with
//! `--discard` (howldrop.exe's outbox holds copies) is deleted.
//!
//! The phone is found among the radio's AWDL peers, each asked `/Discover`
//! (an iPhone's mDNS doesn't reach `awdl0`). It only answers while it
//! listens: unlocked, AirDrop set to Everyone for 10 Minutes, and woken by
//! the BLE advert (from Windows' Bluetooth under howldrop.exe; here with
//! `--ble`). So each file gets a window of tries.
//!
//! Events: `sending NAME DETAIL` as the send gets further, then `sent NAME
//! sent|failed DETAIL`; a scan (the scan-request file appearing) gives
//! `nearby scanning NAMES` and `nearby done NAMES`. The log line `Send:
//! sending NAME` is what holds howldrop.exe's Bluetooth wake on.

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::SocketAddrV6;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use owly_airdrop::driver::sender::{Event, Payload, Sender};
use owly_airdrop::proto::mdns::{RECEIVER_FLAGS, SUPPORTS_DVZIP};
use owly_airdrop::proto::sender::{Config, Outcome};
use owly_awdl::{MacAddr, PeerInfo};
use tokio::sync::watch;

use crate::events::{emit, say};

const TARGET_SUFFIX: &str = ".howldrop-to";
/// Apple's receivers listen here, an AWDL peer without mDNS too (DISC-5).
const AIRDROP_PORT: u16 = 8770;
const SETTLE_LOOKS: u32 = 2;
const STATUS_KEPT: usize = 30;
/// A silent peer mustn't hold the others up for long.
const KNOCK: Duration = Duration::from_secs(3);
/// How long a sweep of the peers gathers answers, for a scan.
const SCAN_SWEEP: Duration = Duration::from_secs(10);
const SCAN_ATTEMPTS: u32 = 3;
const SCAN_PAUSE: Duration = Duration::from_secs(5);
/// How long the iPhone's user has to tap Accept.
const ASK_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Options {
    pub outbox: PathBuf,
    pub discard: bool,
    pub name: String,
    pub to: Option<String>,
    /// How long one file is tried for.
    pub window: Duration,
    pub status: Option<PathBuf>,
    pub nearby: Option<PathBuf>,
    pub scan_request: Option<PathBuf>,
    pub ble: bool,
}

/// The link's index and AWDL peers: where receivers are.
#[derive(Clone)]
pub struct Air {
    pub index: u32,
    pub peers: watch::Receiver<Vec<PeerInfo>>,
}

/// A receiver that answered `/Discover`.
#[derive(Debug, Clone)]
struct Listed {
    mac: MacAddr,
    name: String,
    addr: SocketAddrV6,
}

pub async fn run(air: Air, opts: Options) {
    let sender = match new_sender(&opts.name) {
        Ok(s) => s,
        Err(e) => {
            say(format!("Send: sending isn't available: {e}"));
            return;
        }
    };
    let status = Status::new(opts.status.clone());
    status.write();
    let mut devices: Vec<(MacAddr, String)> = Vec::new();
    write_nearby(opts.nearby.as_deref(), &devices, false);
    say(format!(
        "Send: files put in {} are AirDropped to a nearby iPhone set to Everyone for 10 Minutes.",
        opts.outbox.display()
    ));
    let mut sizes: HashMap<PathBuf, (u64, u32)> = HashMap::new();
    let mut jobs: HashMap<PathBuf, usize> = HashMap::new();
    loop {
        // The daily purge removes empty folders.
        let _ = std::fs::create_dir_all(&opts.outbox);
        if let Some(req) = &opts.scan_request
            && req.exists()
        {
            let _ = std::fs::remove_file(req);
            devices = scan(&air, &sender, &opts, &devices).await;
        }
        for path in waiting(&opts.outbox) {
            let Ok(size) = std::fs::metadata(&path).map(|m| m.len()) else {
                continue;
            };
            let job = *jobs
                .entry(path.clone())
                .or_insert_with(|| status.add(&file_name(&path)));
            let looks = match sizes.get(&path) {
                Some(&(s, n)) if s == size => n + 1,
                _ => 0,
            };
            sizes.insert(path.clone(), (size, looks));
            if looks < SETTLE_LOOKS {
                continue;
            }
            sizes.remove(&path);
            jobs.remove(&path);
            send_one(&air, &sender, &opts, &status, job, &path, size).await;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn new_sender(name: &str) -> io::Result<Sender> {
    let mut config = Config::new(name, crate::receive::random_id()?);
    config.ask_timeout = ASK_TIMEOUT;
    Sender::new(config)
}

async fn send_one(
    air: &Air,
    sender: &Sender,
    opts: &Options,
    status: &Status,
    job: usize,
    path: &Path,
    size: u64,
) {
    let name = file_name(path);
    let to = target_of(path)
        .or_else(|| opts.to.clone())
        .filter(|t| !t.trim().is_empty());
    say(format!(
        "Send: sending {name} ({size} bytes){}",
        to.as_ref().map(|t| format!(" to {t}")).unwrap_or_default()
    ));
    let first = match &to {
        Some(t) => format!("looking for {t}"),
        None => "looking for the iPhone".to_string(),
    };
    status.update(job, Some("sending"), &first);
    emit(&["sending", &name, &first]);
    let progress = |detail: &str| {
        status.update(job, None, detail);
        emit(&["sending", &name, detail]);
    };
    let _advert = advertise(opts.ble).await;
    let (ok, detail) = try_send(air, sender, path, to.as_deref(), opts.window, &progress).await;
    let detail = match (&to, ok) {
        (Some(t), true) => format!("on {t}"),
        _ => detail,
    };
    let where_ = if ok { "sent" } else { "failed" };
    let moved = if opts.discard {
        std::fs::remove_file(path)
    } else {
        let dir = opts.outbox.join(where_);
        std::fs::create_dir_all(&dir).and_then(|()| std::fs::rename(path, free_path(&dir, &name)))
    };
    if let Err(e) = moved {
        say(format!(
            "Send: could not {} {name}: {e}",
            if opts.discard { "delete" } else { "move" }
        ));
    }
    let _ = std::fs::remove_file(sidecar(path));
    status.update(job, Some(where_), &detail);
    emit(&["sent", &name, where_, &detail]);
    say(format!(
        "Send: {name}: {}{}",
        if ok {
            "sent".to_string()
        } else {
            format!("not sent: {detail}")
        },
        if opts.discard {
            String::new()
        } else {
            format!("; moved to {where_}/")
        }
    ));
}

/// Tries one file until it's delivered, refused, or the window closes.
async fn try_send(
    air: &Air,
    sender: &Sender,
    path: &Path,
    to: Option<&str>,
    window: Duration,
    progress: &impl Fn(&str),
) -> (bool, String) {
    let deadline = Instant::now() + window;
    let mut waited = false;
    while Instant::now() < deadline {
        let target = match find(air, sender, to, deadline).await {
            Found::One(t) => t,
            Found::Many(why) => return (false, why),
            Found::None => {
                if !waited {
                    waited = true;
                    let say_ = match to {
                        Some(t) => {
                            format!("waiting for {t}: set its AirDrop to Everyone for 10 Minutes")
                        }
                        None => "no iPhone heard yet: set AirDrop to Everyone for 10 Minutes"
                            .to_string(),
                    };
                    say(format!("Send: {say_}; waiting"));
                    progress(&say_);
                }
                continue;
            }
        };
        say(format!(
            "Send: sending to {:?} at {}",
            target.name, target.mac
        ));
        progress("asking: accept it on the iPhone");
        let (events, mut updates) = tokio::sync::mpsc::unbounded_channel();
        let report = async {
            let mut accepted = false;
            let mut shown = (Instant::now(), u64::MAX);
            while let Some(e) = updates.recv().await {
                match e {
                    Event::Accepted => {
                        accepted = true;
                        progress("accepted; uploading");
                    }
                    Event::Progress(p) if p.total > 0 => {
                        let percent = p.sent * 100 / p.total;
                        if percent != shown.1 && shown.0.elapsed() >= Duration::from_secs(1) {
                            shown = (Instant::now(), percent);
                            progress(&format!("accepted; uploading {percent}%"));
                        }
                    }
                    Event::Progress(_) => {}
                }
            }
            accepted
        };
        let asked = Instant::now();
        let payload = Payload::Files(vec![path.to_path_buf()]);
        let flags = RECEIVER_FLAGS | SUPPORTS_DVZIP;
        let (outcome, accepted) = tokio::join!(
            sender.send(target.addr, &payload, flags, Some(events)),
            report
        );
        match outcome {
            Ok(Outcome::Delivered) => return (true, "on the iPhone".into()),
            Ok(Outcome::Declined) if asked.elapsed() + Duration::from_secs(2) >= ASK_TIMEOUT => {
                return (
                    false,
                    "not accepted on the iPhone in time: send it again and tap Accept".into(),
                );
            }
            Ok(Outcome::Declined) => return (false, "declined on the iPhone".into()),
            Ok(Outcome::Failed(why)) if accepted => return (false, why),
            Err(e) if accepted => return (false, format!("the upload broke off: {e}")),
            // Before the phone took it: its port closes between bursts, and
            // its address rotates. Look again.
            Ok(Outcome::Failed(why)) => say(format!("Send: {why}; trying again")),
            Err(e) => say(format!("Send: {e}; trying again")),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    (
        false,
        format!(
            "the iPhone did not answer within {} minutes: is AirDrop set to Everyone?",
            window.as_secs() / 60
        ),
    )
}

enum Found {
    One(Listed),
    /// Which one is unclear, and why.
    Many(String),
    None,
}

/// Sweeps the AWDL peers until the receiver is picked out, or for up to
/// 30 s (or the deadline).
async fn find(air: &Air, sender: &Sender, to: Option<&str>, deadline: Instant) -> Found {
    let until = deadline.min(Instant::now() + Duration::from_secs(30));
    while Instant::now() < until {
        let listed = sweep(air, sender).await;
        match pick(&listed, to) {
            Found::None => tokio::time::sleep(Duration::from_millis(500)).await,
            found => return found,
        }
    }
    Found::None
}

/// Which of the receivers found is the one to send to.
fn pick(listed: &[Listed], to: Option<&str>) -> Found {
    let mut seen = HashSet::new();
    let listed: Vec<&Listed> = listed.iter().filter(|l| seen.insert(l.mac)).collect();
    match to {
        Some(to) => {
            let matching: Vec<&&Listed> = listed.iter().filter(|l| matches(&l.name, to)).collect();
            match matching.as_slice() {
                [] => Found::None,
                [one] => Found::One((**one).clone()),
                _ => Found::Many(format!(
                    "more than one device answers to \"{to}\": use more of its name"
                )),
            }
        }
        None => match listed.as_slice() {
            [] => Found::None,
            [one] => Found::One((*one).clone()),
            several => {
                let phones: Vec<&&Listed> = several
                    .iter()
                    .filter(|l| l.name.to_lowercase().contains("iphone"))
                    .collect();
                match phones.as_slice() {
                    [one] => Found::One((**one).clone()),
                    _ => Found::Many(
                        "several devices nearby, and none (or more than one) called iPhone".into(),
                    ),
                }
            }
        },
    }
}

/// Whether a receiver's name answers to `to`: in part, either way round of
/// a curly apostrophe, any case.
fn matches(name: &str, to: &str) -> bool {
    let fold = |s: &str| s.replace('\u{2019}', "'").to_lowercase();
    fold(name).contains(fold(to).trim())
}

/// `/Discover` at every AWDL peer the radio hears now, at once.
async fn sweep(air: &Air, sender: &Sender) -> Vec<Listed> {
    let peers: Vec<MacAddr> = air.peers.borrow().iter().map(|p| p.mac).collect();
    let knocks = peers.into_iter().map(|mac| async move {
        let addr = SocketAddrV6::new(mac.link_local(), AIRDROP_PORT, 0, air.index);
        match tokio::time::timeout(KNOCK, sender.discover(addr)).await {
            Ok(Ok(Some(r))) => Some(Listed {
                mac,
                name: r.computer_name,
                addr,
            }),
            _ => None,
        }
    });
    futures_util::future::join_all(knocks)
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// Every device that says its name, over a few sweeps: only those
/// listening now answer.
async fn scan(
    air: &Air,
    sender: &Sender,
    opts: &Options,
    previous: &[(MacAddr, String)],
) -> Vec<(MacAddr, String)> {
    say("Send: looking for nearby devices");
    write_nearby(opts.nearby.as_deref(), previous, true);
    tell_nearby(previous, true);
    let _advert = advertise(opts.ble).await;
    let mut devices: Vec<(MacAddr, String)> = Vec::new();
    for attempt in 0..SCAN_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(SCAN_PAUSE).await;
        }
        let until = Instant::now() + SCAN_SWEEP;
        while Instant::now() < until {
            for l in sweep(air, sender).await {
                if !devices.iter().any(|(m, _)| *m == l.mac) {
                    devices.push((l.mac, l.name));
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if !devices.is_empty() {
            break;
        }
    }
    write_nearby(opts.nearby.as_deref(), &devices, false);
    tell_nearby(&devices, false);
    let names: Vec<&str> = devices.iter().map(|(_, n)| n.as_str()).collect();
    say(format!(
        "Send: nearby: {}",
        if names.is_empty() {
            "nothing answered (is AirDrop set to Everyone?)".to_string()
        } else {
            names.join(", ")
        }
    ));
    devices
}

fn tell_nearby(devices: &[(MacAddr, String)], scanning: bool) {
    let mut f = vec![if scanning { "scanning" } else { "done" }.to_string()];
    f.extend(devices.iter().map(|(_, n)| n.clone()));
    let mut fields = vec!["nearby".to_string()];
    fields.extend(f);
    emit(&fields);
}

/// The latest scan, as JSON for the panel and a `.txt` twin, a name per
/// line, for the relay.
fn write_nearby(path: Option<&Path>, devices: &[(MacAddr, String)], scanning: bool) {
    let Some(path) = path else { return };
    let json = serde_json::json!({
        "time": now(),
        "scanning": scanning,
        "devices": devices.iter().map(|(m, n)| serde_json::json!({"mac": m.to_string(), "name": n})).collect::<Vec<_>>(),
    });
    let mut txt = String::new();
    if scanning {
        txt.push_str("scanning\n");
    }
    for (_, n) in devices {
        txt.push_str(&format!("device\t{n}\n"));
    }
    let _ = replace(path, json.to_string().as_bytes());
    let _ = replace(&path.with_extension("txt"), txt.as_bytes());
}

fn replace(path: &Path, data: &[u8]) -> io::Result<()> {
    let new = PathBuf::from(format!("{}.new", path.display()));
    std::fs::write(&new, data)?;
    std::fs::rename(&new, path)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The latest sends, newest first, as JSON for the panel.
struct Status {
    path: Option<PathBuf>,
    jobs: Mutex<(usize, Vec<Job>)>,
}

#[derive(Clone)]
struct Job {
    serial: usize,
    name: String,
    state: String,
    detail: String,
    time: u64,
}

impl Status {
    fn new(path: Option<PathBuf>) -> Status {
        Status {
            path,
            jobs: Mutex::new((0, Vec::new())),
        }
    }

    fn add(&self, name: &str) -> usize {
        let serial = {
            let mut j = self.jobs.lock().unwrap();
            j.0 += 1;
            let serial = j.0;
            j.1.insert(
                0,
                Job {
                    serial,
                    name: name.into(),
                    state: "waiting".into(),
                    detail: "waiting to send".into(),
                    time: now(),
                },
            );
            j.1.truncate(STATUS_KEPT);
            serial
        };
        self.write();
        serial
    }

    fn update(&self, serial: usize, state: Option<&str>, detail: &str) {
        {
            let mut j = self.jobs.lock().unwrap();
            if let Some(job) = j.1.iter_mut().find(|j| j.serial == serial) {
                if let Some(s) = state {
                    job.state = s.into();
                }
                job.detail = detail.into();
                job.time = now();
            }
        }
        self.write();
    }

    fn write(&self) {
        let Some(path) = &self.path else { return };
        let jobs: Vec<serde_json::Value> = self
            .jobs
            .lock()
            .unwrap()
            .1
            .iter()
            .map(|j| {
                serde_json::json!({
                    "id": format!("{}-{}", j.time, j.serial),
                    "name": j.name, "state": j.state, "detail": j.detail, "time": j.time,
                })
            })
            .collect();
        let _ = replace(
            path,
            serde_json::json!({ "jobs": jobs }).to_string().as_bytes(),
        );
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn sidecar(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}{TARGET_SUFFIX}", path.display()))
}

/// The files waiting in the outbox, by name.
fn waiting(outbox: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(outbox) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            !n.starts_with('.') && !n.ends_with(".partial") && !n.ends_with(TARGET_SUFFIX)
        })
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    out
}

/// The device a file is for, from its sidecar.
fn target_of(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(sidecar(path)).ok()?;
    let name: String = text.trim().chars().take(64).collect();
    (!name.is_empty()).then_some(name)
}

fn free_path(dir: &Path, name: &str) -> PathBuf {
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut path = dir.join(name);
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}{ext}"));
        n += 1;
    }
    path
}

/// BLE-1: the advert that wakes an iPhone's AirDrop, while it's held.
async fn advertise(on: bool) -> Option<crate::bluez::Registration> {
    if !on {
        return None;
    }
    let result = async {
        let mut tag = [0u8; 3];
        getrandom::fill(&mut tag).map_err(|e| io::Error::other(e.to_string()))?;
        crate::bluez::BluezAdvertiser::new()
            .await?
            .advertise(owly_airdrop::proto::ble::Advert::anonymous(tag))
            .await
    }
    .await;
    match result {
        Ok(r) => Some(r),
        Err(e) => {
            say(format!("Bluetooth: not advertising: {e}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(n: u8, name: &str) -> Listed {
        let mac = MacAddr([2, 0, 0, 0, 0, n]);
        Listed {
            mac,
            name: name.into(),
            addr: SocketAddrV6::new(mac.link_local(), AIRDROP_PORT, 0, 1),
        }
    }

    #[test]
    fn the_receiver_is_picked_by_name() {
        let phone = listed(1, "Rocco\u{2019}s iPhone");
        let mac = listed(2, "Rocco\u{2019}s MacBook");
        let both = [phone.clone(), mac.clone()];
        assert!(matches!(pick(&both, Some("rocco's iphone")), Found::One(l) if l.mac == phone.mac));
        assert!(matches!(pick(&both, Some("MacBook")), Found::One(l) if l.mac == mac.mac));
        assert!(matches!(pick(&both, Some("Rocco")), Found::Many(_)));
        assert!(matches!(pick(&both, Some("iPad")), Found::None));
        // Without a name: the only one, or the one called iPhone.
        assert!(matches!(pick(&both, None), Found::One(l) if l.mac == phone.mac));
        assert!(
            matches!(pick(std::slice::from_ref(&mac), None), Found::One(l) if l.mac == mac.mac)
        );
        let two_phones = [phone.clone(), listed(3, "iPhone")];
        assert!(matches!(pick(&two_phones, None), Found::Many(_)));
        assert!(matches!(pick(&[], None), Found::None));
        // The same device twice is one.
        assert!(matches!(
            pick(&[phone.clone(), phone.clone()], None),
            Found::One(_)
        ));
    }

    #[test]
    fn the_outbox_skips_what_isnt_ready() {
        let dir = tempfile::tempdir().unwrap();
        for n in [
            "b.jpg",
            "a.mov",
            ".hidden",
            "c.jpg.partial",
            "a.mov.howldrop-to",
        ] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        std::fs::create_dir(dir.path().join("sent")).unwrap();
        let names: Vec<String> = waiting(dir.path()).iter().map(|p| file_name(p)).collect();
        assert_eq!(names, ["a.mov", "b.jpg"]);
        std::fs::write(
            dir.path().join("a.mov.howldrop-to"),
            " Rocco\u{2019}s iPhone \n",
        )
        .unwrap();
        assert_eq!(
            target_of(&dir.path().join("a.mov")).as_deref(),
            Some("Rocco\u{2019}s iPhone")
        );
        assert_eq!(target_of(&dir.path().join("b.jpg")), None);
        std::fs::write(dir.path().join("sent").join("a.mov"), b"x").unwrap();
        assert_eq!(
            free_path(&dir.path().join("sent"), "a.mov"),
            dir.path().join("sent").join("a-1.mov")
        );
    }

    #[test]
    fn nearby_is_written_for_the_panel_and_the_relay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("send-nearby.json");
        let mac = MacAddr([2, 0, 0, 0, 0, 1]);
        write_nearby(Some(&path), &[(mac, "Phone".into())], true);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["scanning"], true);
        assert_eq!(json["devices"][0]["name"], "Phone");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("send-nearby.txt")).unwrap(),
            "scanning\ndevice\tPhone\n"
        );
    }

    #[test]
    fn status_keeps_the_latest_sends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("send-status.json");
        let status = Status::new(Some(path.clone()));
        let a = status.add("a.jpg");
        status.update(a, Some("sent"), "on the iPhone");
        status.add("b.jpg");
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["jobs"][0]["name"], "b.jpg");
        assert_eq!(json["jobs"][1]["state"], "sent");
        assert_eq!(json["jobs"][1]["detail"], "on the iPhone");
    }
}
