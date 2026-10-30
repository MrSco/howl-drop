//! What the agent prints: log lines, and with `--events` the `@howldrop`
//! events howldrop.exe reads (wsl_win.cpp, transfers.cpp): `@howldrop`,
//! then tab-separated fields, none holding a tab or a line break.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static ON: AtomicBool = AtomicBool::new(false);

/// Prints events from now on.
pub fn enable() {
    ON.store(true, Ordering::Relaxed);
}

/// A log line, at once (stdout is line-buffered).
pub fn say(line: impl AsRef<str>) {
    println!("{}", line.as_ref());
}

/// An event, when they're on.
pub fn emit<S: AsRef<str>>(fields: &[S]) {
    if ON.load(Ordering::Relaxed) {
        println!("{}", line(fields));
    }
}

/// The event line for `fields`: whitespace runs that hold a tab or a line
/// break become one space, as announce.event and howl_ask.event did.
pub fn line<S: AsRef<str>>(fields: &[S]) -> String {
    let mut out = String::from("@howldrop");
    for f in fields {
        out.push('\t');
        out.push_str(&clean(f.as_ref()));
    }
    out
}

/// How long `bytes` took, for the log: "14.7 MB in 6.3 s (2.3 MB/s)".
/// Under a second there's no speed: it would mostly be the clock's error.
pub fn took(bytes: u64, elapsed: Duration) -> String {
    if elapsed < Duration::from_secs(1) {
        return format!("{} in under a second", size(bytes));
    }
    let secs = elapsed.as_secs_f64();
    format!(
        "{} in {secs:.1} s ({}/s)",
        size(bytes),
        size((bytes as f64 / secs) as u64)
    )
}

/// Bytes in decimal units, as the iPhone and Explorer's details show them.
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} bytes")
    }
}

fn clean(field: &str) -> String {
    let lines: Vec<&str> = field.lines().collect();
    lines.join(" ").replace('\t', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_lose_tabs_and_line_breaks() {
        assert_eq!(
            line(&[
                "ask",
                "ab12",
                "Rocco\u{2019}s\tiPhone",
                "1",
                "5",
                "a\nb.jpg"
            ]),
            "@howldrop\task\tab12\tRocco\u{2019}s iPhone\t1\t5\ta b.jpg"
        );
        assert_eq!(line::<&str>(&[]), "@howldrop");
    }

    #[test]
    fn times_read_plainly() {
        assert_eq!(
            took(14_712_229, Duration::from_millis(6300)),
            "14.7 MB in 6.3 s (2.3 MB/s)"
        );
        assert_eq!(
            took(621_178, Duration::from_millis(700)),
            "621 KB in under a second"
        );
        assert_eq!(took(900, Duration::ZERO), "900 bytes in under a second");
        assert_eq!(
            took(2_000_000, Duration::from_secs(1)),
            "2.0 MB in 1.0 s (2.0 MB/s)"
        );
        assert_eq!(size(2_600), "3 KB");
        assert_eq!(size(3_200_000_000), "3.2 GB");
    }
}
