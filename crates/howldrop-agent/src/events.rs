//! What the agent prints: log lines, and with `--events` the `@howldrop`
//! events howldrop.exe reads (wsl_win.cpp, transfers.cpp): `@howldrop`,
//! then tab-separated fields, none holding a tab or a line break.

use std::sync::atomic::{AtomicBool, Ordering};

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
}
