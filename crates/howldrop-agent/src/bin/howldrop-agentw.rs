//! Windows: the scheduled task's program (setup.rs). Runs howldrop-agent.exe
//! beside it, with the same arguments and no console window, and ends with
//! it: Task Scheduler opens a console window for a console program it runs
//! for the logged-on user, for as long as it runs. This one is a Windows
//! program, so it has none, and the agent is started without one.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    /// CREATE_NO_WINDOW.
    const NO_WINDOW: u32 = 0x0800_0000;
    let Some(agent) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("howldrop-agent.exe")))
    else {
        return ExitCode::FAILURE;
    };
    let status = Command::new(agent)
        .args(std::env::args_os().skip(1))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(NO_WINDOW)
        .status();
    match status.ok().and_then(|s| s.code()) {
        Some(code) => ExitCode::from(code as u8),
        None => ExitCode::FAILURE,
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("howldrop-agentw is howldrop-agent's launcher on Windows.");
    ExitCode::FAILURE
}
