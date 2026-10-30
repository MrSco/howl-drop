//! A radio session's runtime directory: what a later process needs to undo
//! the session. owlyd's is `/run/owly`:
//!
//! | File | Content |
//! |---|---|
//! | `journal.json`, `journal.lock` | the recovery [journal](crate::journal), written before each change |
//! | `hostapd.log` | hostapd's output |
//! | `hostapd.conf` | the hidden AP's configuration, readable by root only |
//!
//! Files are replaced by rename, so a reader never sees half of one. The
//! link and its peers are owlyd's D-Bus API's to say, not files'.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The runtime directory of the system's radio session.
pub const RUN_DIR: &str = "/run/owly";

#[derive(Debug, Clone)]
pub struct RunDir {
    path: PathBuf,
}

impl RunDir {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        RunDir { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    /// Creates the directory, readable by everyone.
    pub fn create(&self) -> io::Result<()> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(&self.path)
    }

    /// Replaces `name` with `contents`.
    pub fn write(&self, name: &str, contents: &[u8]) -> io::Result<()> {
        self.create()?;
        let tmp = self.file(&format!(".{name}.{}", std::process::id()));
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        fs::rename(&tmp, self.file(name))
    }

    pub fn read(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.file(name)).ok()
    }

    pub fn remove(&self, name: &str) {
        let _ = fs::remove_file(self.file(name));
    }

    pub fn write_json<T: Serialize>(&self, name: &str, value: &T) -> io::Result<()> {
        let mut json = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
        json.push(b'\n');
        self.write(name, &json)
    }

    pub fn read_json<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        serde_json::from_str(&self.read(name)?).ok()
    }
}

/// A process, told apart from a later one with its pid by its start time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Process {
    pub pid: u32,
    /// Field 22 of `/proc/<pid>/stat`, in clock ticks since boot.
    pub start: u64,
}

impl Process {
    pub fn me() -> Option<Process> {
        Process::of(std::process::id())
    }

    pub fn of(pid: u32) -> Option<Process> {
        Some(Process {
            pid,
            start: start_time(pid)?,
        })
    }

    pub fn alive(&self) -> bool {
        // A zombie has exited; only its parent hasn't noticed yet.
        start_time(self.pid) == Some(self.start) && !is_zombie(self.pid)
    }

    pub fn signal(&self, signal: Signal) -> bool {
        self.alive() && kill(Pid::from_raw(self.pid as i32), signal).is_ok()
    }

    /// SIGTERM, then SIGKILL if it's still running after `grace`. Returns
    /// whether it's gone.
    pub fn stop(&self, grace: Duration) -> bool {
        if !self.signal(Signal::SIGTERM) {
            return !self.alive();
        }
        if self.wait(grace) {
            return true;
        }
        self.signal(Signal::SIGKILL);
        self.wait(Duration::from_secs(5))
    }

    /// Waits up to `limit` for it to exit.
    pub fn wait(&self, limit: Duration) -> bool {
        let end = Instant::now() + limit;
        while self.alive() {
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        true
    }
}

/// The fields after the command name, which may itself hold spaces and
/// parentheses.
fn stat_fields(pid: u32) -> Option<Vec<String>> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    Some(rest.split_whitespace().map(String::from).collect())
}

fn start_time(pid: u32) -> Option<u64> {
    // Field 22; the fields after the name start at 3.
    stat_fields(pid)?.get(22 - 3)?.parse().ok()
}

fn is_zombie(pid: u32) -> bool {
    stat_fields(pid).is_some_and(|f| f.first().is_some_and(|s| s == "Z"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recycled_pid_is_another_process() {
        let me = Process::me().unwrap();
        assert!(me.alive());
        let other = Process {
            start: me.start + 1,
            ..me
        };
        assert!(!other.alive());
    }

    #[test]
    fn stop_ends_a_process() {
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let p = Process::of(child.id()).unwrap();
        assert!(p.alive());
        // Reaped by a thread, as a holder's parent would.
        let reaper = std::thread::spawn(move || {
            let mut child = child;
            child.wait()
        });
        assert!(p.stop(Duration::from_secs(2)));
        reaper.join().unwrap().unwrap();
        assert!(!p.alive());
    }

    #[test]
    fn files_are_replaced_whole() {
        let dir = tempfile::tempdir().unwrap();
        let run = RunDir::new(dir.path());
        run.write("hostapd.log", b"1 2").unwrap();
        assert_eq!(run.read("hostapd.log").unwrap(), "1 2");
        run.write("hostapd.log", b"").unwrap();
        assert_eq!(run.read("hostapd.log").unwrap(), "");
        let leftovers: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1);
    }
}
