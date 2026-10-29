//! The radio: owly-radio's injection backend, set up from the agent's
//! options rather than /etc/owly/radio.toml, held by this process alone (no
//! owlyd). It picks the card and its mode from the quirks table, writes
//! each change to its recovery journal, and takes it all back on the way
//! down.

use std::sync::Arc;

use owly_radio::config::Config;
use owly_radio::injection::{AWDL_IFACE, Injection};
use owly_radio::radio::{Backend, Capabilities, Link};
use owly_radio::rundir::RunDir;
use owly_radio::sys::{Ops, System};

use crate::events::{emit, say};

/// The radio options, as the add-on and wsl-run.sh give them.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The Wi-Fi interface; else the best one the probe finds.
    pub station: Option<String>,
    pub allow_untested: bool,
    /// AirDrop channels in the order to try (6, 44, 149); else the card's.
    pub channels: Option<Vec<u8>>,
    /// Accept the phone's Block Ack sessions; else as the card's quirk says.
    pub aggregation: Option<bool>,
    /// The regulatory domain; else US where the kernel's is the world's.
    pub country: Option<String>,
    pub tx_rate: Option<String>,
    pub run_dir: String,
}

/// The backend, with its journal of what a crashed run left undone.
pub struct Radio {
    backend: Injection,
    probe: Capabilities,
}

impl Radio {
    pub fn new(opts: &Options) -> Result<Radio, String> {
        let ops = Arc::new(System);
        let country = opts.country.clone().or_else(|| {
            // An unset domain (00, or 99 for a driver's own) leaves 5 GHz
            // receive-only, and the phone never sees us there.
            match ops.regdom().as_deref() {
                None | Some("00") | Some("99") => {
                    say("The regulatory domain is unset, which leaves 5 GHz receive-only. Setting it to US for the window.");
                    Some("US".into())
                }
                Some(_) => None,
            }
        });
        let config = Config {
            station: opts.station.clone(),
            allow_untested: opts.allow_untested,
            reconnect_after: None,
            country,
            channels: opts.channels.clone(),
            aggregation: opts.aggregation,
            tx_rate: opts.tx_rate.clone(),
            hostapd: None,
            tx_amsdu: false,
        };
        let forced = config.country.is_some() && opts.country.is_none();
        let settings = config.settings(RunDir::new(&opts.run_dir))?;
        let backend = Injection::new(ops, settings);
        let mut probe = backend.probe();
        if forced {
            // The probe judged the channels by the world's domain, which
            // the window replaces: the window's log says which it gets.
            probe.channel = None;
            probe
                .issues
                .retain(|i| !i.starts_with("AirDrop runs on channel"));
        }
        Ok(Radio { backend, probe })
    }

    /// What the probe found, for the log and the PC.
    pub fn report(&self) {
        let p = &self.probe;
        let name = |o: &Option<String>| o.clone().unwrap_or_else(|| "?".into());
        say(format!(
            "Wi-Fi card: {} (driver {}, {}), {}, {} mode{}.",
            name(&p.station),
            name(&p.driver),
            name(&p.device),
            p.status
                .map(|s| format!("{s:?}").to_lowercase())
                .unwrap_or_else(|| "unknown".into()),
            p.mode.map(|m| m.to_string()).unwrap_or_else(|| "no".into()),
            p.channel
                .map(|c| format!(", channel {c}"))
                .unwrap_or_default(),
        ));
        for m in &p.missing {
            say(format!("Missing ({}): {}", m.id, m.say));
        }
        for issue in &p.issues {
            say(format!("Note: {issue}"));
        }
        emit(&[
            "adapter",
            p.driver.as_deref().unwrap_or(""),
            &p.status
                .map(|s| format!("{s:?}").to_lowercase())
                .unwrap_or_default(),
            &p.mode.map(|m| m.to_string()).unwrap_or_default(),
            p.device.as_deref().unwrap_or(""),
        ]);
    }

    /// Why no window can start here, when the probe found no card to use.
    pub fn unusable(&self) -> Option<String> {
        if self.probe.hardware && self.probe.station.is_some() {
            return None;
        }
        Some(
            self.probe
                .missing
                .first()
                .map(|m| m.say.clone())
                .unwrap_or_else(|| "No Wi-Fi card here can carry the AirDrop window.".into()),
        )
    }

    /// The Wi-Fi interface the window runs on.
    pub fn station(&self) -> String {
        self.probe.station.clone().unwrap_or_else(|| "?".into())
    }

    /// Gives Wi-Fi back from what a run that died left in the journal.
    pub async fn recover(&mut self) {
        if let Err(e) = self.backend.recover().await {
            say(format!("Undoing the last run's radio changes: {e}"));
        }
    }

    pub async fn up(&mut self) -> Result<Link, String> {
        let link = self.backend.up().await.map_err(|e| e.message)?;
        tune_tcp().await;
        Ok(link)
    }

    pub async fn down(&mut self) {
        if let Err(e) = self.backend.down().await {
            say(format!("Giving the card back: {e}"));
        }
    }
}

/// TCP on awdl0 resends after 20 ms rather than 200: a frame the radio
/// missed costs a slot, not a fifth of a second. No quickack: an ACK per
/// segment takes the air the phone sends on.
async fn tune_tcp() {
    let out = tokio::process::Command::new("ip")
        .args([
            "-6",
            "route",
            "add",
            "fe80::/64",
            "dev",
            AWDL_IFACE,
            "metric",
            "128",
            "rto_min",
            "20ms",
        ])
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => {}
        Ok(o) => say(format!(
            "TCP on {AWDL_IFACE} keeps the 200 ms retransmit floor: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => say(format!(
            "TCP on {AWDL_IFACE} keeps the 200 ms retransmit floor: ip: {e}"
        )),
    }
}

/// The channels option: "149 44 6", "149,44", or empty/auto for the card's.
pub fn channels(option: &str) -> Result<Option<Vec<u8>>, String> {
    let words: Vec<&str> = option.split([' ', ',']).filter(|w| !w.is_empty()).collect();
    if words.is_empty() || words == ["auto"] {
        return Ok(None);
    }
    words
        .iter()
        .map(|w| match w.parse::<u8>() {
            Ok(c @ (6 | 44 | 149)) => Ok(c),
            _ => Err(format!("channel {w} isn't one of AirDrop's (6, 44, 149)")),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// The aggregation option: on, off, or auto for the card's quirk.
pub fn aggregation(option: &str) -> Result<Option<bool>, String> {
    match option.trim().to_lowercase().as_str() {
        "" | "auto" => Ok(None),
        "on" | "true" | "yes" | "1" => Ok(Some(true)),
        "off" | "false" | "no" | "0" => Ok(Some(false)),
        other => Err(format!("aggregation is on, off or auto, not {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse() {
        assert_eq!(channels("").unwrap(), None);
        assert_eq!(channels(" auto ").unwrap(), None);
        assert_eq!(channels("149 44,6").unwrap(), Some(vec![149, 44, 6]));
        assert!(channels("36").is_err());
        assert_eq!(aggregation("Auto").unwrap(), None);
        assert_eq!(aggregation("on").unwrap(), Some(true));
        assert_eq!(aggregation("False").unwrap(), Some(false));
        assert!(aggregation("maybe").is_err());
    }
}
