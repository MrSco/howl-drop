//! Giving Wi-Fi back as the lease holder's user, for `owly radio up` and
//! `owly test-card`.
//!
//! owlyd brings the station's connection back up after exclusive mode, but
//! as root. A password kept per user (KDE's KWallet, GNOME Keyring, with
//! `psk-flags` 1) is only handed to that user's secret agent, so root's
//! request fails with "Secrets were required" and Wi-Fi stays down. A
//! client, which runs in the user's session, notes the Wi-Fi connections
//! that are up before it takes a lease, and once the lease is over brings
//! up, as itself, any that owlyd didn't bring back. Without NetworkManager
//! there's nothing to note.

use std::process::{Command, Stdio};

/// A Wi-Fi connection NetworkManager has up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub uuid: String,
    pub device: String,
}

/// The Wi-Fi connections that are up now.
pub fn active() -> Vec<Connection> {
    let Ok(out) = Command::new("nmcli")
        .args([
            "-t",
            "-f",
            "UUID,TYPE,DEVICE",
            "connection",
            "show",
            "--active",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    parse_active(&String::from_utf8_lossy(&out.stdout))
}

/// `nmcli -t`'s lines: none of these fields can hold a colon.
fn parse_active(text: &str) -> Vec<Connection> {
    text.lines()
        .filter_map(|line| {
            let [uuid, kind, device] = line.split(':').collect::<Vec<_>>()[..] else {
                return None;
            };
            (kind == "802-11-wireless" && !device.is_empty()).then(|| Connection {
                uuid: uuid.into(),
                device: device.into(),
            })
        })
        .collect()
}

/// Brings up, as this process's user, each of `before` that isn't up again.
/// Call it once owlyd has given Wi-Fi back: after `Release` returns, or
/// once the lease's end is signalled.
pub async fn restore(before: Vec<Connection>) {
    if before.is_empty() {
        return;
    }
    let _ = tokio::task::spawn_blocking(move || {
        let now = active();
        for c in before.iter().filter(|c| !now.contains(c)) {
            let up = Command::new("nmcli")
                .args(["-w", "10", "connection", "up", "uuid", &c.uuid])
                .args(["ifname", &c.device])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            match up {
                Ok(s) if s.success() => {
                    tracing::info!("brought {} back up on {}", c.uuid, c.device)
                }
                _ => tracing::warn!("{} didn't come back up on {}", c.uuid, c.device),
            }
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_wifi_connections_on_a_device() {
        let text = "\
8d2c9f8e-1b4f-4d47-9a45-3b0c2a6f1e10:802-11-wireless:wlp1s0
0e1c2b3a-4d5e-6f70-8192-a3b4c5d6e7f8:802-3-ethernet:enp3s0
a1b2c3d4-e5f6-4789-abcd-ef0123456789:loopback:lo
f0e1d2c3-b4a5-4697-8879-6a5b4c3d2e1f:802-11-wireless:
";
        assert_eq!(
            parse_active(text),
            [Connection {
                uuid: "8d2c9f8e-1b4f-4d47-9a45-3b0c2a6f1e10".into(),
                device: "wlp1s0".into(),
            }]
        );
    }
}
