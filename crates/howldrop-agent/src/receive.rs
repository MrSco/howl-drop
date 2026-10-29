//! Receiving: owly-airdrop's receiver on `awdl0`, announced over mDNS, with
//! HowlDrop's policy for who may send, and what it leaves for deliver.py.
//!
//! Who may send:
//! - everyone: every `/Ask` is accepted as it comes;
//! - ask: a known contact is accepted at once, anyone else waits for the
//!   PC's user: an `ask` event, then `ID.accept` or `ID.decline` in the ask
//!   folder (a Windows folder, which has no inotify: polled);
//! - contacts: only a known contact, proven by its Apple ID's certificate
//!   and record, may send, or see this receiver.
//!
//! A decline answers 401, which an iPhone shows as "Declined".
//!
//! Files land in the staging folder (owly-airdrop writes into a hidden
//! folder there and moves each finished item in), with a note of who sent
//! each in `.howldrop-meta/<name>.json`, which deliver.py moves on to the
//! share or the PC's inbox. A link becomes an inert `.url` shortcut there.
//!
//! Events (`--events`), tab-separated after `@howldrop`, as howl_ask.py
//! printed them:
//!   ask ID SENDER COUNT BYTES NAME...       waiting for Accept / Decline
//!   asked ID accepted|declined|timeout      how that ended
//!   incoming ID SENDER COUNT BYTES NAME...  accepted: on its way
//!   receiving ID BYTES TOTAL                the upload so far
//!   stored ID NAME...                       its files are written
//!   failed ID WHY                           it broke off

use std::collections::HashMap;
use std::io;
use std::net::SocketAddrV6;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use owly_airdrop::driver::mdns::{Mdns, Registration};
use owly_airdrop::driver::receiver::{
    Options as ReceiverOptions, Receiver, ReceiverEvent, Request,
};
use owly_airdrop::driver::tls::Identity;
use owly_airdrop::proto::mdns::{self, RECEIVER_FLAGS, Service};
use owly_airdrop::proto::messages::{AskRequest, Offer};
use owly_airdrop::proto::receiver::{Config, DiscoverGate};
use owly_radio::radio::Link;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::contacts::{Contacts, Outcome};
use crate::events::{emit, say};

/// Where we receive: the port omdrop's firewall rule opens on `awdl0`.
pub const PORT: u16 = 8771;
/// Names shown in an event; the count says how many there are.
const NAMES_SHOWN: usize = 20;
const POLL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Everyone,
    Ask,
    Contacts,
}

impl std::str::FromStr for Policy {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "everyone" => Ok(Policy::Everyone),
            "ask" => Ok(Policy::Ask),
            "contacts" => Ok(Policy::Contacts),
            _ => Err(format!("--receive is everyone, ask or contacts, not {s:?}")),
        }
    }
}

pub struct Options {
    pub name: String,
    pub model: String,
    pub staging: PathBuf,
    pub identity: Identity,
    pub policy: Policy,
    pub ask_dir: Option<PathBuf>,
    /// How long the PC's user has to answer.
    pub ask_timeout: Duration,
    pub contacts: Arc<Contacts>,
    pub verbose: bool,
}

/// One transfer, from its `/Ask` on.
struct Transfer {
    /// Ours, for the PC: howldrop.exe takes hex ids (decide()).
    tag: String,
    sender: String,
    total: u64,
    /// Ends the wait for the PC's user when the sender leaves.
    withdrawn: Option<oneshot::Sender<()>>,
}

type Transfers = Arc<Mutex<HashMap<u64, Transfer>>>;

/// The receiver and its mDNS announcement, until dropped.
pub struct Running {
    tasks: Vec<JoinHandle<()>>,
    _registration: Registration,
}

impl Drop for Running {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

pub async fn start(link: &Link, opts: Options) -> io::Result<Running> {
    std::fs::create_dir_all(&opts.staging)?;
    let opts = Arc::new(opts);
    let mut config = Config::new(opts.name.clone());
    config.model_name = opts.model.clone();
    // Our own wait ends first, so a timeout is ours to report.
    config.ask_timeout = opts.ask_timeout + Duration::from_secs(5);
    if opts.policy == Policy::Contacts {
        let contacts = opts.contacts.clone();
        config.discover_gate = Some(DiscoverGate(Arc::new(move |request| {
            let ok = contacts.may_discover(request.sender_record.as_deref());
            if !ok {
                say("Receive: discovery withheld (contacts only): sender is not a contact");
            }
            ok
        })));
    }
    let addr = SocketAddrV6::new(link.addr, PORT, 0, link.index);
    let mut options = ReceiverOptions::new(config, opts.staging.clone());
    options.progress = true;
    let (receiver, mut events) = Receiver::bind(addr, &opts.identity, options).await?;
    let serve = tokio::spawn(receiver.run());

    let service_id = random_id()?;
    let burst = link.announce.map(|a| mdns::Burst {
        count: a.count,
        interval: a.interval,
        pause: a.pause,
    });
    let registration = Mdns::new(link.index)
        .with_burst(burst)
        .register(Service {
            instance: service_id.clone(),
            host: format!("howldrop-{service_id}"),
            port: PORT,
            addr: link.addr,
            flags: RECEIVER_FLAGS,
        })
        .await?;
    say(format!(
        "Receive: listening on [{}]:{PORT} as {:?}, {}.",
        link.addr,
        opts.name,
        match opts.policy {
            Policy::Everyone => "everyone nearby can send".to_string(),
            Policy::Ask => format!(
                "contacts ({}) are accepted at once, anyone else is asked about on the PC",
                opts.contacts.len()
            ),
            Policy::Contacts => format!("contacts only ({})", opts.contacts.len()),
        }
    ));

    let transfers: Transfers = Arc::default();
    let handle = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            on_event(event, &opts, &transfers);
        }
    });
    Ok(Running {
        tasks: vec![serve, handle],
        _registration: registration,
    })
}

fn on_event(event: ReceiverEvent, opts: &Arc<Options>, transfers: &Transfers) {
    let tag_of = |id: u64| transfers.lock().unwrap().get(&id).map(|t| t.tag.clone());
    match event {
        ReceiverEvent::Discovered { peer, .. } => {
            if opts.verbose {
                say(format!("Receive: seen by {peer}"));
            }
        }
        ReceiverEvent::Request(request) => {
            let (sender, names, total) = describe(&request.ask);
            let (withdrawn, gone) = oneshot::channel();
            let tag = random_id().unwrap_or_else(|_| format!("{:x}", request.id));
            transfers.lock().unwrap().insert(
                request.id,
                Transfer {
                    tag: tag.clone(),
                    sender: sender.clone(),
                    total,
                    withdrawn: Some(withdrawn),
                },
            );
            say(format!(
                "Receive: ask from {sender:?}: {} item(s), {total} bytes: {}",
                names.len(),
                names.join(", ")
            ));
            let opts = opts.clone();
            tokio::spawn(async move {
                answer(request, &opts, tag, sender, names, total, gone).await;
            });
        }
        ReceiverEvent::Withdrawn { id } => {
            if let Some(t) = transfers.lock().unwrap().get_mut(&id)
                && let Some(w) = t.withdrawn.take()
            {
                let _ = w.send(());
            }
        }
        ReceiverEvent::Progress { id, bytes } => {
            let Some((tag, total)) = transfers
                .lock()
                .unwrap()
                .get(&id)
                .map(|t| (t.tag.clone(), t.total))
            else {
                return;
            };
            emit(&["receiving", &tag, &bytes.to_string(), &total.to_string()]);
        }
        ReceiverEvent::Urls { id, urls } => {
            let sender = transfers
                .lock()
                .unwrap()
                .remove(&id)
                .map(|t| t.sender)
                .unwrap_or_default();
            for url in urls {
                match store_link(&opts.staging, &url, &sender) {
                    Ok(name) => say(format!("Receive: link stored as {name}")),
                    Err(e) => say(format!("Receive: couldn't store a link: {e}")),
                }
            }
        }
        ReceiverEvent::Received { id, paths } => {
            let t = transfers.lock().unwrap().remove(&id);
            let (tag, sender) = t.map(|t| (t.tag, t.sender)).unwrap_or_default();
            let mut names = Vec::new();
            for p in &paths {
                note_tree(&opts.staging, p, &sender);
                names.push(file_name(p));
            }
            say(format!("Receive: stored {}", names.join(", ")));
            let mut fields = vec!["stored".to_string(), tag];
            fields.extend(names);
            emit(&fields);
        }
        ReceiverEvent::Failed { id, reason } => {
            say(format!("Receive: the transfer failed: {reason}"));
            if let Some(tag) = tag_of(id) {
                emit(&["failed", &tag, &reason]);
            }
            transfers.lock().unwrap().remove(&id);
        }
    }
}

/// Accepts or declines one `/Ask` as the policy says.
async fn answer(
    request: Request,
    opts: &Options,
    tag: String,
    sender: String,
    names: Vec<String>,
    total: u64,
    gone: oneshot::Receiver<()>,
) {
    let known = || {
        let contacts = opts.contacts.clone();
        let record = request.ask.sender_record.clone();
        let cert = request.peer_certificate.clone();
        async move {
            if contacts.is_empty() {
                return Outcome::Unknown;
            }
            let (outcome, detail) = tokio::task::spawn_blocking(move || {
                contacts.explain(record.as_deref(), cert.as_deref())
            })
            .await
            .unwrap_or((Outcome::Unknown, String::new()));
            if outcome != Outcome::Accept {
                let detail = if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                };
                say(format!(
                    "Receive: not a known contact: {}{detail}",
                    outcome.why()
                ));
            }
            outcome
        }
    };
    let shown: Vec<String> = names.iter().take(NAMES_SHOWN).cloned().collect();
    let card = |what: &str| {
        let mut f = vec![
            what.to_string(),
            tag.clone(),
            sender.clone(),
            names.len().to_string(),
            total.to_string(),
        ];
        f.extend(shown.iter().cloned());
        f
    };
    let accept = match opts.policy {
        Policy::Everyone => true,
        Policy::Contacts => {
            let outcome = known().await;
            if outcome == Outcome::Accept {
                say(format!("Receive: ask accepted: {}", outcome.why()));
            } else {
                say(format!(
                    "Receive: ask refused (contacts only): {}",
                    outcome.why()
                ));
            }
            outcome == Outcome::Accept
        }
        Policy::Ask => {
            if known().await == Outcome::Accept {
                say("Receive: ask from a known contact: accepted without asking");
                true
            } else {
                emit(&card("ask"));
                let outcome = match &opts.ask_dir {
                    Some(dir) => wait_decision(dir, &tag, opts.ask_timeout, gone).await,
                    None => "timeout",
                };
                emit(&["asked", &tag, outcome]);
                say(format!("Receive: ask {outcome} on the PC"));
                outcome == "accepted"
            }
        }
    };
    if accept {
        request.accept();
        emit(&card("incoming"));
    } else {
        request.decline();
    }
}

/// `accepted`, `declined` or `timeout`: the first of `TAG.accept` or
/// `TAG.decline` to appear in `dir`, removed once read; or the sender
/// leaving, or the time running out.
async fn wait_decision(
    dir: &Path,
    tag: &str,
    limit: Duration,
    mut gone: oneshot::Receiver<()>,
) -> &'static str {
    let deadline = tokio::time::Instant::now() + limit;
    let files = [
        ("accepted", dir.join(format!("{tag}.accept"))),
        ("declined", dir.join(format!("{tag}.decline"))),
    ];
    loop {
        for (outcome, path) in &files {
            if path.exists() {
                let _ = std::fs::remove_file(path);
                return outcome;
            }
        }
        tokio::select! {
            () = tokio::time::sleep(POLL) => {}
            _ = &mut gone => return "timeout",
        }
        if tokio::time::Instant::now() >= deadline {
            return "timeout";
        }
    }
}

/// (sender, names, total bytes) of an `/Ask`. A links ask names its
/// addresses.
fn describe(ask: &AskRequest) -> (String, Vec<String>, u64) {
    let sender = if ask.sender_computer_name.trim().is_empty() {
        "Someone".to_string()
    } else {
        ask.sender_computer_name.clone()
    };
    match &ask.offer {
        Offer::Files(files) => (
            sender,
            files.iter().map(|f| f.name.clone()).collect(),
            files.iter().filter_map(|f| f.size).sum(),
        ),
        Offer::Urls(urls) => (sender, urls.clone(), 0),
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Notes who sent `path`, and each file in it if it's a folder, for
/// deliver.py and announce.py: `.howldrop-meta/<name>.json`.
fn note_tree(staging: &Path, path: &Path, sender: &str) {
    if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for e in entries.flatten() {
                note_tree(staging, &e.path(), sender);
            }
        }
        return;
    }
    if let Err(e) = note(staging, &file_name(path), sender, None) {
        say(format!(
            "Receive: couldn't note who sent {}: {e}",
            file_name(path)
        ));
    }
}

fn note(staging: &Path, name: &str, sender: &str, url: Option<&str>) -> io::Result<()> {
    let meta = staging.join(".howldrop-meta");
    std::fs::create_dir_all(&meta)?;
    let body = serde_json::json!({ "sender": sender, "url": url });
    std::fs::write(meta.join(format!("{name}.json")), body.to_string())
}

/// Writes a received link as an inert `.url` shortcut, named from its host
/// and last path segment, and returns the name. Nothing opens it.
fn store_link(staging: &Path, url: &str, sender: &str) -> io::Result<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = path.split(['?', '#']).next().unwrap_or("");
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let stem: Vec<&str> = [host, last].into_iter().filter(|p| !p.is_empty()).collect();
    let stem = safe_name(&stem.join("-"), "link");
    let name = free_name(staging, &stem, ".url");
    // Written aside and moved in, so deliver.py never sees half of it.
    let partial = staging.join(format!("{name}.partial"));
    std::fs::write(&partial, format!("[InternetShortcut]\nURL={url}\n"))?;
    note(staging, &name, sender, Some(url))?;
    std::fs::rename(&partial, staging.join(&name))?;
    Ok(name)
}

/// A file name from text the sender chose: no path separators, controls or
/// characters Windows refuses, no leading dot, at most 100 characters.
fn safe_name(text: &str, fallback: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// `stem + ext`, or `stem-N + ext` beside one already there.
fn free_name(dir: &Path, stem: &str, ext: &str) -> String {
    let mut name = format!("{stem}{ext}");
    let mut n = 1;
    while dir.join(&name).exists() || dir.join(format!("{name}.partial")).exists() {
        name = format!("{stem}-{n}{ext}");
        n += 1;
    }
    name
}

/// Twelve hex digits.
pub fn random_id() -> io::Result<String> {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_become_shortcuts_with_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let name = store_link(
            dir.path(),
            "https://example.com/a/photo.png?x=1",
            "Rocco\u{2019}s iPhone",
        )
        .unwrap();
        assert_eq!(name, "example.com-photo.png.url");
        let text = std::fs::read_to_string(dir.path().join(&name)).unwrap();
        assert_eq!(
            text,
            "[InternetShortcut]\nURL=https://example.com/a/photo.png?x=1\n"
        );
        let note: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                dir.path()
                    .join(".howldrop-meta")
                    .join(format!("{name}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(note["sender"], "Rocco\u{2019}s iPhone");
        assert_eq!(note["url"], "https://example.com/a/photo.png?x=1");
        // A second one beside it, and a host alone.
        assert_eq!(
            store_link(dir.path(), "https://example.com/a/photo.png", "").unwrap(),
            "example.com-photo.png-1.url"
        );
        assert_eq!(
            store_link(dir.path(), "https://example.org", "").unwrap(),
            "example.org.url"
        );
        assert_eq!(safe_name("../..\\x:y", "link"), "_.._x_y");
        assert_eq!(safe_name("...", "link"), "link");
    }

    #[test]
    fn received_files_are_noted() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Album");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("a.jpg"), b"a").unwrap();
        std::fs::write(dir.path().join("b.mov"), b"b").unwrap();
        note_tree(dir.path(), &folder, "Phone");
        note_tree(dir.path(), &dir.path().join("b.mov"), "Phone");
        let meta = dir.path().join(".howldrop-meta");
        assert!(meta.join("a.jpg.json").exists());
        assert!(meta.join("b.mov.json").exists());
    }

    #[tokio::test]
    async fn the_pc_answers_by_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_keep, gone) = oneshot::channel();
        let wait = tokio::spawn({
            let dir = dir.path().to_path_buf();
            async move { wait_decision(&dir, "ab12", Duration::from_secs(5), gone).await }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        std::fs::write(dir.path().join("ab12.decline"), "\n").unwrap();
        assert_eq!(wait.await.unwrap(), "declined");
        assert!(!dir.path().join("ab12.decline").exists());

        let (_keep, gone) = oneshot::channel();
        assert_eq!(
            wait_decision(dir.path(), "cd34", Duration::from_millis(300), gone).await,
            "timeout"
        );
        let (left, gone) = oneshot::channel();
        let _ = left.send(());
        assert_eq!(
            wait_decision(dir.path(), "ef56", Duration::from_secs(5), gone).await,
            "timeout"
        );
    }

    #[test]
    fn asks_describe_their_offer() {
        let ask = AskRequest {
            transfer_id: None,
            transfer_type: None,
            sender_computer_name: " ".into(),
            sender_model_name: String::new(),
            sender_id: String::new(),
            bundle_id: String::new(),
            sender_record: None,
            file_icon: None,
            offer: Offer::Urls(vec!["https://example.com".into()]),
            described: String::new(),
        };
        assert_eq!(
            describe(&ask),
            ("Someone".into(), vec!["https://example.com".into()], 0)
        );
    }
}
