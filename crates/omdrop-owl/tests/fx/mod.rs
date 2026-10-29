//! The fixture both test files share: a fake owlyd, on the fake radio and
//! a private bus, and the hooks and helper installed as packaged, under the
//! test seam.

#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use owly_radio::fake::{self, Fake};
use owly_radio::sys::parse_if_inet6;
use owlyd::service::{self, Options};

pub const HOOKS: [&str; 5] = ["probe", "up", "tick", "down", "peers"];
pub const PHONE: &str = "86:08:04:8a:2b:f4";

pub struct Fx {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
    bus: Child,
    owlyd: Option<Owlyd>,
}

/// owlyd on the fake radio, in a thread of the test.
struct Owlyd {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Owlyd {
    fn start(root: &Path, address: &str) -> Owlyd {
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let (root, address) = (root.to_path_buf(), address.to_string());
        let (ready_tx, ready) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let serve = service::serve(
                    Fake::new(&root),
                    fake::probe,
                    Options {
                        bus: Some(address.clone()),
                        polkit: false,
                    },
                    async move {
                        let _ = stopped.await;
                    },
                );
                let ready = async {
                    let conn = owlyd::api::connect(Some(&address)).await.unwrap();
                    let dbus = zbus::fdo::DBusProxy::new(&conn).await.unwrap();
                    while !dbus
                        .name_has_owner(owlyd::api::NAME.try_into().unwrap())
                        .await
                        .unwrap()
                    {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    let _ = ready_tx.send(());
                };
                let (served, ()) = tokio::join!(serve, ready);
                served.unwrap();
            });
        });
        ready
            .recv_timeout(Duration::from_secs(10))
            .expect("owlyd took its name");
        Owlyd {
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}

impl Drop for Owlyd {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Fx {
    pub fn new() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        for d in ["hooks", "conf", "owly-fake", "run/awdl-discoverable"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        fs::set_permissions(
            root.join("run/awdl-discoverable"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        // One binary under each hook's name, as installed.
        for h in HOOKS {
            symlink(env!("CARGO_BIN_EXE_omdrop-owl"), root.join("hooks").join(h)).unwrap();
        }
        // And as omdrop-discoverable, beside a stand-in for omdrop-awdl's
        // announcer that notes each burst it's asked for.
        fs::create_dir_all(root.join("omdrop")).unwrap();
        symlink(
            env!("CARGO_BIN_EXE_omdrop-owl"),
            root.join("omdrop/omdrop-discoverable"),
        )
        .unwrap();
        let announcer = root.join("omdrop/awdl-airdrop-adv.py");
        fs::write(
            &announcer,
            "#!/bin/sh\necho \"$*\" >> \"$(dirname \"$0\")/announced\"\n",
        )
        .unwrap();
        fs::set_permissions(&announcer, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(root.join("owly-fake/peers"), format!("{PHONE}\n")).unwrap();
        let socket = root.join("bus");
        let address = format!("unix:path={}", socket.display());
        let bus = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--nopidfile"])
            .arg(format!("--address={address}"))
            .stdout(Stdio::null())
            .spawn()
            .expect("dbus-daemon");
        let end = Instant::now() + Duration::from_secs(10);
        while !socket.exists() {
            assert!(Instant::now() < end, "the bus didn't start");
            std::thread::sleep(Duration::from_millis(20));
        }
        fs::write(root.join("owly-fake/bus"), &address).unwrap();
        let owlyd = Some(Owlyd::start(&root, &address));
        Fx {
            _dir: dir,
            root,
            bus,
            owlyd,
        }
    }

    /// Without owlyd on the bus.
    pub fn stop_owlyd(&mut self) {
        self.owlyd = None;
    }

    /// A hook as omdrop-radio runs it.
    pub fn command(&self, hook: &str, args: &[&str]) -> Command {
        let mut c = Command::new(self.root.join("hooks").join(hook));
        c.args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/usr/sbin")
            .env("LANG", "C.UTF-8")
            .env("OMDROP_CONF", self.root.join("conf"))
            .env("OMDROP_RUN_DIR", self.root.join("run/awdl-discoverable"))
            .env("OMDROP_RADIO_HOOKS", "1")
            .env("OMDROP_RADIO_TEST_ROOT", &self.root)
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        c
    }

    /// Runs a hook to completion within `limit`, pipes and all: a hook
    /// that leaves a process holding its output open fails here.
    pub fn run_within(&self, hook: &str, args: &[&str], limit: Duration) -> Output {
        within(self.command(hook, args), &format!("{hook} {args:?}"), limit)
    }

    pub fn run(&self, hook: &str, args: &[&str]) -> Output {
        self.run_within(hook, args, Duration::from_secs(60))
    }

    /// `omdrop-discoverable` as Omdrop runs it through pkexec, which clears
    /// the environment (here, all but the test seam), to completion, pipes
    /// and all: Omdrop reads `start`'s output until it closes.
    pub fn helper(&self, args: &[&str]) -> Output {
        let mut c = Command::new(self.root.join("omdrop/omdrop-discoverable"));
        c.args(args)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("OMDROP_RADIO_TEST_ROOT", &self.root)
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        within(
            c,
            &format!("omdrop-discoverable {args:?}"),
            Duration::from_secs(60),
        )
    }

    /// The arguments of each announcement burst the helper ran.
    pub fn announced(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("omdrop/announced"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    /// The helper's window supervisor's pid, as `start` recorded it.
    pub fn supervisor(&self) -> Option<i32> {
        fs::read_to_string(self.root.join("run/omdrop-owl/supervisor"))
            .ok()?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }

    pub fn fake(&self, name: &str, content: &str) {
        fs::write(self.root.join("owly-fake").join(name), content).unwrap();
    }

    pub fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("owly-fake/calls"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    /// `awdl0` as omdrop-radio checks it: present, with a usable
    /// link-local address in `proc/net/if_inet6`.
    pub fn awdl0_up(&self) -> bool {
        let if_inet6 = fs::read_to_string(self.root.join("proc/net/if_inet6")).unwrap_or_default();
        self.root.join("sys/class/net/awdl0").exists()
            && matches!(parse_if_inet6(&if_inet6, "awdl0"), Some((_, false)))
    }

    /// The window's lease, as `up` recorded it.
    pub fn lease(&self) -> Option<u64> {
        fs::read_to_string(self.root.join("run/omdrop-owl/lease"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// Core's own run directory, which no hook may write.
    pub fn cores_files(&self) -> Vec<PathBuf> {
        fs::read_dir(self.root.join("run/awdl-discoverable"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect()
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        // A helper window a failed test left open.
        if let Some(pid) = self.supervisor() {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
        self.owlyd = None;
        let _ = self.bus.kill();
        let _ = self.bus.wait();
    }
}

fn within(mut c: Command, what: &str, limit: Duration) -> Output {
    let child = c.spawn().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    rx.recv_timeout(limit)
        .unwrap_or_else(|_| panic!("{what} took over {limit:?}"))
        .unwrap()
}

pub fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

pub fn last_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .last()
        .unwrap_or("")
        .to_string()
}
