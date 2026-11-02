//! Windows: what the agent needs set up once, as administrator, so that
//! howldrop.exe can start it from then on without Windows asking
//! (`--setup`, docs/native-windows.md, milestone 7):
//!
//! 1. the agent, its launcher (howldrop-agentw.exe), wintun.dll and its
//!    firmware in Program Files\HowlDrop, where only an administrator can
//!    change them;
//! 2. a scheduled task, `\HowlDrop\AirDrop`, that runs it from there with
//!    the user's full rights (the Wintun adapter needs them), at normal
//!    priority and without a console window, its arguments from a file of
//!    the user's ([`args_file`]);
//! 3. Windows' own WinUSB driver (winusb.inf, "WinUsb Device") for each
//!    supported stick on another, as Device Manager's "Let me pick" puts
//!    it on: no driver package or certificate of ours.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::events::say;

pub use winusb::cycle_sticks;

/// The task's name, in a folder of its own.
pub const TASK: &str = r"\HowlDrop\AirDrop";
/// Where the agent's arguments are, under %LOCALAPPDATA%.
const ARGS: &str = r"HowlDrop\native\agent.args";

/// The installed agent's folder.
fn install_dir() -> Result<PathBuf, String> {
    std::env::var_os("ProgramFiles")
        .map(|p| PathBuf::from(p).join("HowlDrop"))
        .ok_or_else(|| "no Program Files folder".into())
}

/// The file the task's agent reads its arguments from.
pub fn args_file() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join(ARGS))
        .ok_or_else(|| "no %LOCALAPPDATA%".into())
}

/// All three steps; what went wrong, else nothing.
pub fn run() -> Result<(), String> {
    let installed = install()?;
    // Through its launcher, which has no console window to show.
    register_task(&installed.join("howldrop-agentw.exe"))?;
    winusb::install_for_sticks()
}

/// Copies this program, wintun.dll and firmware\ into Program Files.
fn install() -> Result<PathBuf, String> {
    let from = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .ok_or("where this program is")?;
    let to = install_dir()?;
    if same_dir(&from, &to) {
        say(format!("Setup: running from {} already.", to.display()));
        return Ok(to);
    }
    let copy = |name: &Path| -> Result<(), String> {
        let (a, b) = (from.join(name), to.join(name));
        if let Some(dir) = b.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::copy(&a, &b).map(drop).map_err(|e| {
            format!(
                "copying {} to {}: {e} (is AirDrop on this PC still on?)",
                a.display(),
                b.display()
            )
        })
    };
    copy(Path::new("howldrop-agent.exe"))?;
    copy(Path::new("howldrop-agentw.exe"))?;
    copy(Path::new("wintun.dll"))?;
    for f in ["mt7662_rom_patch.bin", "mt7662.bin"] {
        copy(&Path::new("firmware").join(f))?;
    }
    say(format!("Setup: the agent is in {}.", to.display()));
    Ok(to)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The task, for this user, as the XML schtasks takes.
fn task_xml(exe: &Path, args: &Path, user: &str) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>HowlDrop's AirDrop radio, started by howldrop.exe when AirDrop on this PC is on.</Description>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <AllowHardTerminate>true</AllowHardTerminate>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{exe}"</Command>
      <Arguments>--args-file "{args}"</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        user = esc(user),
        exe = esc(&exe.display().to_string()),
        args = esc(&args.display().to_string()),
    )
}

/// The task, made or replaced. Priority 4 is a normal one: a task's own
/// default (7) runs it below normal, slow and late for AWDL's timing.
fn register_task(exe: &Path) -> Result<(), String> {
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(d), Ok(u)) => format!(r"{d}\{u}"),
        (_, Ok(u)) => u,
        _ => return Err("who the user is".into()),
    };
    let xml = task_xml(exe, &args_file()?, &user);
    // UTF-16 with its byte order mark, as schtasks reads task XML.
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    let file = std::env::temp_dir().join("howldrop-task.xml");
    std::fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    let out = Command::new("schtasks")
        .args(["/create", "/f", "/tn", TASK, "/xml"])
        .arg(&file)
        .output()
        .map_err(|e| format!("schtasks: {e}"));
    let _ = std::fs::remove_file(&file);
    let out = out?;
    if !out.status.success() {
        return Err(format!(
            "registering the task: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    say(format!("Setup: the task {TASK} runs it for {user}."));
    Ok(())
}

/// The arguments in `file`, one a line, for the task's run; refused when a
/// folder they name is outside the user's own profile, since they come
/// from a file the user's programs can write and the agent runs with full
/// rights.
pub fn read_args(file: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    // Every line an argument, an empty one too (--senders with none).
    let args: Vec<String> = text
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    let profile = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .ok_or("no %USERPROFILE%")?;
    check_folders(&args, &profile)?;
    Ok(args)
}

/// The options that name a folder the agent writes into.
const FOLDERS: [&str; 8] = [
    "--staging",
    "--deliver",
    "--identity",
    "--ask-dir",
    "--outbox",
    "--scan-request",
    "--send-status",
    "--nearby",
];

fn check_folders(args: &[String], profile: &Path) -> Result<(), String> {
    let root = format!(
        "{}\\",
        profile
            .to_string_lossy()
            .trim_end_matches('\\')
            .to_lowercase()
    );
    for pair in args.windows(2) {
        if !FOLDERS.contains(&pair[0].as_str()) {
            continue;
        }
        let path = Path::new(&pair[1]);
        let inside = path.is_absolute()
            && !path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            && path.to_string_lossy().to_lowercase().starts_with(&root);
        if !inside {
            return Err(format!(
                "{} {} isn't in {}",
                pair[0],
                path.display(),
                profile.display()
            ));
        }
    }
    Ok(())
}

mod winusb {
    //! Windows' own WinUSB for the stick, as Device Manager installs it
    //! when told "Let me pick" and "WinUsb Device": the device's class set
    //! to USBDevice, winusb.inf's driver list built for it, its WinUSB
    //! model chosen and installed.

    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
    use windows_sys::Win32::Foundation::{GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::core::GUID;

    use crate::events::say;

    /// GUID_DEVCLASS_USBDEVICE, winusb.inf's class.
    const USB_DEVICE: GUID = GUID::from_u128(0x88bae032_5a81_49f0_bc3d_a4ff138216d6);
    /// winusb.inf's model, as Device Manager lists it.
    const MODEL: &str = "WinUsb Device";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn text(w: &[u16]) -> String {
        let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
        String::from_utf16_lossy(&w[..end])
    }

    fn error(what: &str) -> String {
        // SAFETY: reads the thread's last error.
        #[allow(unsafe_code)]
        let code = unsafe { GetLastError() };
        format!("{what} (error 0x{code:08x})")
    }

    /// A device's registry property as text: the first of a list.
    #[allow(unsafe_code)]
    fn property(set: HDEVINFO, dev: &SP_DEVINFO_DATA, which: u32) -> Option<Vec<String>> {
        let mut buf = vec![0u16; 1024];
        let mut size = 0u32;
        // SAFETY: buf holds its stated size in bytes.
        let ok = unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                set,
                dev,
                which,
                null_mut(),
                buf.as_mut_ptr().cast(),
                (buf.len() * 2) as u32,
                &mut size,
            )
        };
        if ok == 0 {
            return None;
        }
        buf.truncate(size as usize / 2);
        Some(
            buf.split(|&c| c == 0)
                .filter(|s| !s.is_empty())
                .map(String::from_utf16_lossy)
                .collect(),
        )
    }

    /// The supported sticks present, each on WinUSB afterwards.
    #[allow(unsafe_code)]
    pub fn install_for_sticks() -> Result<(), String> {
        let ids: Vec<String> = mt7612u::USB_IDS
            .iter()
            .map(|(v, p)| format!("USB\\VID_{v:04X}&PID_{p:04X}"))
            .collect();
        // SAFETY: SetupAPI as documented; the set is destroyed on the way
        // out, and every struct is sized as it asks.
        unsafe {
            let set = SetupDiGetClassDevsW(
                null(),
                wide("USB").as_ptr(),
                null_mut(),
                DIGCF_ALLCLASSES | DIGCF_PRESENT,
            );
            if set == INVALID_HANDLE_VALUE as HDEVINFO {
                return Err(error("listing USB devices"));
            }
            let mut result = Ok(());
            let mut found = 0;
            for i in 0.. {
                let mut dev: SP_DEVINFO_DATA = std::mem::zeroed();
                dev.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
                if SetupDiEnumDeviceInfo(set, i, &mut dev) == 0 {
                    break;
                }
                let hw = property(set, &dev, SPDRP_HARDWAREID).unwrap_or_default();
                let Some(id) = hw
                    .iter()
                    .find(|h| ids.iter().any(|i| h.to_uppercase().starts_with(i.as_str())))
                else {
                    continue;
                };
                found += 1;
                let service = property(set, &dev, SPDRP_SERVICE)
                    .and_then(|s| s.into_iter().next())
                    .unwrap_or_default();
                if service.eq_ignore_ascii_case("WinUSB") {
                    say(format!("Setup: {id} is on WinUSB already."));
                    continue;
                }
                say(format!(
                    "Setup: {id} is on {}; putting it on WinUSB.",
                    if service.is_empty() {
                        "no driver"
                    } else {
                        &service
                    }
                ));
                if let Err(e) = install(set, &mut dev) {
                    result = Err(format!("WinUSB for {id}: {e}"));
                    continue;
                }
                say(format!("Setup: {id} is on WinUSB."));
                // The driver it had leaves the chip answering nothing: as
                // unplugging it and plugging it in.
                match cycle(set, &dev) {
                    Ok(()) => say(format!("Setup: {id} restarted.")),
                    Err(e) => say(format!(
                        "Setup: {id} couldn't be restarted ({e}): unplug it and plug it in."
                    )),
                }
            }
            SetupDiDestroyDeviceInfoList(set);
            if found == 0 {
                say("Setup: no supported stick is plugged in; plug it in and run this again.");
            }
            result
        }
    }

    /// Each supported stick restarted as if unplugged and plugged in (its
    /// hub port cycled): how many. Needs an administrator.
    #[allow(unsafe_code)]
    pub fn cycle_sticks() -> Result<usize, String> {
        let ids: Vec<String> = mt7612u::USB_IDS
            .iter()
            .map(|(v, p)| format!("USB\\VID_{v:04X}&PID_{p:04X}"))
            .collect();
        // SAFETY: as in install_for_sticks.
        unsafe {
            let set = SetupDiGetClassDevsW(
                null(),
                wide("USB").as_ptr(),
                null_mut(),
                DIGCF_ALLCLASSES | DIGCF_PRESENT,
            );
            if set == INVALID_HANDLE_VALUE as HDEVINFO {
                return Err(error("listing USB devices"));
            }
            let mut cycled = Ok(0);
            for i in 0.. {
                let mut dev: SP_DEVINFO_DATA = std::mem::zeroed();
                dev.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
                if SetupDiEnumDeviceInfo(set, i, &mut dev) == 0 {
                    break;
                }
                let hw = property(set, &dev, SPDRP_HARDWAREID).unwrap_or_default();
                if hw
                    .iter()
                    .any(|h| ids.iter().any(|i| h.to_uppercase().starts_with(i.as_str())))
                {
                    cycled = match (cycled, cycle(set, &dev)) {
                        (Ok(n), Ok(())) => Ok(n + 1),
                        (Ok(_), Err(e)) | (Err(e), _) => Err(e),
                    };
                }
            }
            SetupDiDestroyDeviceInfoList(set);
            cycled
        }
    }

    /// The device's port on its hub cycled (IOCTL_USB_HUB_CYCLE_PORT): the
    /// hub drops it and finds it again, as when it's plugged in.
    #[allow(unsafe_code)]
    unsafe fn cycle(set: HDEVINFO, dev: &SP_DEVINFO_DATA) -> Result<(), String> {
        use windows_sys::Win32::Devices::Usb::{
            GUID_DEVINTERFACE_USB_HUB, IOCTL_USB_HUB_CYCLE_PORT, USB_CYCLE_PORT_PARAMS,
        };
        use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows_sys::Win32::System::IO::DeviceIoControl;
        // SAFETY: the caller's set and device; buffers sized as stated, the
        // interface detail in u32s for its alignment; handles closed.
        unsafe {
            // A USB device's address is its port on the hub.
            let mut port = 0u32;
            if SetupDiGetDeviceRegistryPropertyW(
                set,
                dev,
                SPDRP_ADDRESS,
                null_mut(),
                (&raw mut port).cast(),
                4,
                null_mut(),
            ) == 0
            {
                return Err(error("its port"));
            }
            let mut parent = 0u32;
            let mut hub = [0u16; 512];
            if CM_Get_Parent(&mut parent, dev.DevInst, 0) != CR_SUCCESS
                || CM_Get_Device_IDW(parent, hub.as_mut_ptr(), hub.len() as u32, 0) != CR_SUCCESS
            {
                return Err("its hub".into());
            }
            let hubs = SetupDiGetClassDevsW(
                &GUID_DEVINTERFACE_USB_HUB,
                hub.as_ptr(),
                null_mut(),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            );
            if hubs == INVALID_HANDLE_VALUE as HDEVINFO {
                return Err(error("its hub's interface"));
            }
            let mut iface: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
            iface.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            let mut detail = vec![0u32; 512];
            let d = detail
                .as_mut_ptr()
                .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            (*d).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            let found = SetupDiEnumDeviceInterfaces(
                hubs,
                null(),
                &GUID_DEVINTERFACE_USB_HUB,
                0,
                &mut iface,
            ) != 0
                && SetupDiGetDeviceInterfaceDetailW(
                    hubs,
                    &iface,
                    d,
                    (detail.len() * 4) as u32,
                    null_mut(),
                    null_mut(),
                ) != 0;
            let handle = if found {
                CreateFileW(
                    (*d).DevicePath.as_ptr(),
                    GENERIC_WRITE,
                    FILE_SHARE_WRITE,
                    null(),
                    OPEN_EXISTING,
                    0,
                    null_mut(),
                )
            } else {
                INVALID_HANDLE_VALUE
            };
            SetupDiDestroyDeviceInfoList(hubs);
            if handle == INVALID_HANDLE_VALUE {
                return Err(error("opening its hub"));
            }
            let mut params = USB_CYCLE_PORT_PARAMS {
                ConnectionIndex: port,
                StatusReturned: 0,
            };
            let mut returned = 0u32;
            let ok = DeviceIoControl(
                handle,
                IOCTL_USB_HUB_CYCLE_PORT,
                (&raw const params).cast(),
                size_of::<USB_CYCLE_PORT_PARAMS>() as u32,
                (&raw mut params).cast(),
                size_of::<USB_CYCLE_PORT_PARAMS>() as u32,
                &mut returned,
                null_mut(),
            );
            let failed = (ok == 0).then(|| error("cycling its port"));
            CloseHandle(handle);
            failed.map_or(Ok(()), Err)
        }
    }

    #[allow(unsafe_code)]
    unsafe fn install(set: HDEVINFO, dev: &mut SP_DEVINFO_DATA) -> Result<(), String> {
        // SAFETY: the caller's set and device; structs sized as asked.
        unsafe {
            // Its class as winusb.inf's, so that file's models are its own.
            let guid = USB_DEVICE;
            let as_text = wide(&format!(
                "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
                guid.data1,
                guid.data2,
                guid.data3,
                guid.data4[0],
                guid.data4[1],
                guid.data4[2],
                guid.data4[3],
                guid.data4[4],
                guid.data4[5],
                guid.data4[6],
                guid.data4[7]
            ));
            if SetupDiSetDeviceRegistryPropertyW(
                set,
                dev,
                SPDRP_CLASSGUID,
                as_text.as_ptr().cast(),
                (as_text.len() * 2) as u32,
            ) == 0
            {
                return Err(error("setting its class"));
            }
            dev.ClassGuid = guid;

            let mut params: SP_DEVINSTALL_PARAMS_W = std::mem::zeroed();
            params.cbSize = size_of::<SP_DEVINSTALL_PARAMS_W>() as u32;
            if SetupDiGetDeviceInstallParamsW(set, dev, &mut params) == 0 {
                return Err(error("reading its install settings"));
            }
            let windows = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
            let inf = wide(&format!(r"{windows}\INF\winusb.inf"));
            if inf.len() > params.DriverPath.len() {
                return Err("winusb.inf's path is too long".into());
            }
            params.DriverPath[..inf.len()].copy_from_slice(&inf);
            params.Flags |= DI_ENUMSINGLEINF | DI_QUIETINSTALL;
            params.FlagsEx |= DI_FLAGSEX_ALLOWEXCLUDEDDRVS;
            if SetupDiSetDeviceInstallParamsW(set, dev, &params) == 0 {
                return Err(error("pointing it at winusb.inf"));
            }
            if SetupDiBuildDriverInfoList(set, dev, SPDIT_CLASSDRIVER) == 0 {
                return Err(error("reading winusb.inf"));
            }
            let mut chosen = None;
            for i in 0.. {
                let mut drv: SP_DRVINFO_DATA_V2_W = std::mem::zeroed();
                drv.cbSize = size_of::<SP_DRVINFO_DATA_V2_W>() as u32;
                if SetupDiEnumDriverInfoW(set, dev, SPDIT_CLASSDRIVER, i, &mut drv) == 0 {
                    break;
                }
                if text(&drv.Description) == MODEL {
                    chosen = Some(drv);
                    break;
                }
            }
            let Some(mut drv) = chosen else {
                return Err(format!("winusb.inf has no {MODEL:?}"));
            };
            if SetupDiSetSelectedDriverW(set, dev, &mut drv) == 0 {
                return Err(error("choosing WinUSB"));
            }
            let mut reboot = 0;
            if DiInstallDevice(null_mut(), set, dev, &drv, 0, &mut reboot) == 0 {
                return Err(error("installing WinUSB"));
            }
            if reboot != 0 {
                say("Setup: Windows wants a restart to finish.");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_runs_the_agent_at_normal_priority() {
        let xml = task_xml(
            Path::new(r"C:\Program Files\HowlDrop\howldrop-agent.exe"),
            Path::new(r"C:\Users\A & B\AppData\Local\HowlDrop\native\agent.args"),
            r"PC\A & B",
        );
        assert!(
            xml.contains(r#"<Command>"C:\Program Files\HowlDrop\howldrop-agent.exe"</Command>"#)
        );
        assert!(xml.contains("<Arguments>--args-file \"C:\\Users\\A &amp; B\\"));
        assert!(xml.contains(r"<UserId>PC\A &amp; B</UserId>"));
        assert!(xml.contains("<Priority>4</Priority>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
    }

    #[test]
    fn empty_arguments_stay() {
        // In the profile, where the task's files are.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agent.args");
        std::fs::write(&file, "--senders\n\n--name\r\nPC\n").unwrap();
        assert_eq!(read_args(&file).unwrap(), ["--senders", "", "--name", "PC"]);
    }

    #[test]
    fn folders_stay_in_the_profile() {
        let profile = Path::new(r"C:\Users\Me");
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(
            check_folders(
                &args(&[
                    "--name",
                    "PC",
                    "--deliver",
                    r"C:\Users\me\Downloads\HowlDrop"
                ]),
                profile
            )
            .is_ok()
        );
        assert!(check_folders(&args(&["--deliver", r"C:\Windows\System32"]), profile).is_err());
        assert!(check_folders(&args(&["--staging", r"C:\Users\Me\..\Other"]), profile).is_err());
        assert!(check_folders(&args(&["--outbox", "relative"]), profile).is_err());
        assert!(check_folders(&args(&["--outbox", r"C:\Users\MeToo\x"]), profile).is_err());
        // A name that looks like a folder is only a name.
        assert!(check_folders(&args(&["--name", r"C:\Windows"]), profile).is_ok());
    }
}
