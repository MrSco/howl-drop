//! Windows: what the stick needs once, as administrator (`--setup`, which
//! howldrop.exe runs with Windows asking when the agent says the stick
//! isn't ready): Windows' own WinUSB driver (winusb.inf, "WinUsb Device")
//! for each supported stick on another, as Device Manager's "Let me pick"
//! puts it on, with no driver package or certificate of ours; then the
//! stick restarted by cycling its hub port, since the driver it had leaves
//! the chip answering nothing. The agent itself runs without an
//! administrator (docs/native-windows.md).

pub use winusb::cycle_sticks;

/// WinUSB for the sticks; what went wrong, else nothing.
pub fn run() -> Result<(), String> {
    winusb::install_for_sticks()
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
