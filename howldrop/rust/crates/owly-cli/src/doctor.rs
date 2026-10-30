//! `owly doctor`: owlyd's `Probe()`, for people.

use std::process::ExitCode;

use owly_radio::probe;
use owly_radio::radio::Missing;
use owly_radio::rundir::{RUN_DIR, RunDir};
use owlyd::api;
use serde_json::Value;

/// The report, from owlyd, or from here with a word about owlyd when it
/// can't be reached.
pub async fn report(bus: Option<&str>) -> Value {
    let from_owlyd = async {
        let conn = api::connect(bus).await?;
        api::radio(&conn).await?.probe().await
    };
    match from_owlyd.await {
        Ok(json) => serde_json::from_str(&json).unwrap_or(Value::Null),
        Err(e) => {
            let mut caps = probe::from_config(RunDir::new(RUN_DIR));
            caps.missing.insert(
                0,
                Missing {
                    id: "backend",
                    say: format!(
                        "Owly's radio service, owlyd, can't be reached ({e}), so this report is from here. `systemctl enable --now owlyd` starts it."
                    ),
                },
            );
            serde_json::to_value(&caps).expect("capabilities serialize")
        }
    }
}

pub async fn run(bus: Option<&str>, json: bool) -> Result<ExitCode, String> {
    let report = report(bus).await;
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        print!("{}", text(&report));
    }
    Ok(if ready(&report) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

pub fn ready(report: &Value) -> bool {
    report["missing"].as_array().is_some_and(|m| m.is_empty())
}

/// The report as `owly doctor` prints it.
pub fn text(r: &Value) -> String {
    let s = |v: &Value| v.as_str().unwrap_or("?").to_string();
    let mut out = String::new();
    let mut line = |k: &str, v: String| out.push_str(&format!("{k:<9} {v}\n"));
    if !r["station"].is_null() {
        let mut card = s(&r["driver"]);
        if let Some(d) = r["device"].as_str() {
            card.push_str(&format!(", {d}"));
        }
        let status = r["status"]
            .as_str()
            .map(|st| format!(", {st}"))
            .unwrap_or_default();
        line("station", format!("{} ({card}){status}", s(&r["station"])));
    }
    if let Some(mode) = r["mode"].as_str() {
        let channel = r["channel"]
            .as_u64()
            .map(|c| format!(", channel {c}"))
            .unwrap_or_default();
        line("mode", format!("{mode}{channel}"));
    }
    let c = &r["card"];
    if c.is_object() {
        let yn = |v: &Value| {
            if v.as_bool() == Some(true) {
                "yes"
            } else {
                "no"
            }
        };
        line(
            "card",
            format!(
                "{}: ad-hoc {}, AP {}, active monitor {}, monitor beside the station {}",
                s(&c["phy"]),
                yn(&c["ibss"]),
                yn(&c["ap"]),
                yn(&c["active_monitor"]),
                yn(&c["monitor_beside_station"]),
            ),
        );
        let managed = if c["self_managed_reg"].as_bool() == Some(true) {
            " (the driver's own)"
        } else {
            ""
        };
        line("regdom", format!("{}{managed}", s(&c["regdom"])));
        let channels: Vec<String> = c["channels"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|ch| {
                let state = ch["why"].as_str().unwrap_or("ok");
                format!("{} {state}", ch["channel"])
            })
            .collect();
        line("channels", channels.join(", "));
    }
    let missing = r["missing"].as_array().cloned().unwrap_or_default();
    if missing.is_empty() {
        out.push_str("ready\n");
    }
    for m in missing {
        out.push_str(&format!("{}: {}\n", s(&m["id"]), s(&m["say"])));
    }
    for i in r["issues"].as_array().into_iter().flatten() {
        out.push_str(&format!("note: {}\n", s(i)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_reads_as_text() {
        let r = serde_json::json!({
            "station": "wlan0",
            "driver": "mt7925e",
            "device": "14c3:0717",
            "status": "tested",
            "mode": "shared",
            "channel": 44,
            "card": {
                "phy": "phy0", "ibss": true, "ap": true, "active_monitor": false,
                "monitor_beside_station": true, "ibss_beside_station": true,
                "self_managed_reg": false, "regdom": "DE",
                "channels": [
                    {"channel": 6, "freq": 2437, "transmit": true},
                    {"channel": 149, "freq": 5745, "transmit": false, "why": "no-ir"}
                ]
            },
            "hardware": true,
            "missing": [],
            "issues": ["Slow."]
        });
        assert_eq!(
            text(&r),
            "station   wlan0 (mt7925e, 14c3:0717), tested\n\
             mode      shared, channel 44\n\
             card      phy0: ad-hoc yes, AP yes, active monitor no, monitor beside the station yes\n\
             regdom    DE\n\
             channels  6 ok, 149 no-ir\n\
             ready\n\
             note: Slow.\n"
        );
        assert!(ready(&r));
    }
}
