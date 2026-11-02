//! howldrop.exe's line to the agent on Windows (`--pipe`): a named pipe it
//! makes before starting the agent as administrator, which Windows starts
//! without the caller's standard handles. Standard output and error go to
//! the pipe, and the pipe going (howldrop.exe disconnecting it, or ending)
//! stops the agent. It's watched with PeekNamedPipe, never read: a read
//! waiting on the handle would hold up every write to it.

use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::sync::OnceLock;
use std::time::Duration;

use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;

/// How often the pipe is looked at.
const WATCH: Duration = Duration::from_millis(300);

/// The pipe, open as long as the process, once attached.
static PIPE: OnceLock<File> = OnceLock::new();

/// Connects to the pipe `name` and makes it standard output and error.
#[allow(unsafe_code)]
pub fn attach(name: &str) -> io::Result<()> {
    let file = File::options().read(true).write(true).open(name)?;
    let file = PIPE.get_or_init(|| file);
    let handle = file.as_raw_handle();
    // SAFETY: the handle stays open for the rest of the process (PIPE is
    // never dropped), and std reads the standard handles anew on each write.
    unsafe {
        SetStdHandle(STD_OUTPUT_HANDLE, handle);
        SetStdHandle(STD_ERROR_HANDLE, handle);
    }
    Ok(())
}

/// The pipe, when `attach` connected one.
pub fn attached() -> Option<&'static File> {
    PIPE.get()
}

/// Calls `gone` once the pipe is no longer connected, from a thread.
pub fn watch(file: &'static File, gone: impl FnOnce() + Send + 'static) -> io::Result<()> {
    std::thread::Builder::new()
        .name("pipe".into())
        .spawn(move || {
            while connected(file) {
                std::thread::sleep(WATCH);
            }
            gone();
        })
        .map(drop)
}

#[allow(unsafe_code)]
fn connected(file: &File) -> bool {
    let mut available = 0u32;
    // SAFETY: an open pipe handle; nothing is read, only the count asked.
    unsafe {
        PeekNamedPipe(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        ) != 0
    }
}
