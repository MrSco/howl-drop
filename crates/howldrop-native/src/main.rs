//! HowlDrop on Windows without WSL (docs/native-windows.md, milestone 5):
//! the user-space MT7612U driver as the radio (crates/mt7612u), owly-awdl
//! on it, a Wintun adapter as the link, and owly-airdrop's receiver on that,
//! taking files from everyone. Run as administrator (Wintun makes its
//! adapter), with the stick on WinUSB and wintun.dll beside the program.
//!
//!   howldrop-native FIRMWARE_DIR STAGING_DIR [NAME] [SECONDS]

#[cfg(windows)]
mod link;
#[cfg(windows)]
mod run;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    run::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("howldrop-native runs on Windows only.");
    std::process::exit(1);
}
