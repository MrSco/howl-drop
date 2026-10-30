//! rtnetlink's changes on a real kernel, without root: the test re-runs
//! itself under `unshare --user --map-root-user --net --mount`, with sysfs
//! remounted to show that namespace's interfaces, and there changes a dummy
//! interface the way the radio changes the monitor and `awdl0`.

use std::process::Command;

use owly_radio::radio::MacAddr;
use owly_radio::rtnl::Rtnl;
use owly_radio::sys::{self, Ops, System};

const INNER: &str = "OWLY_RTNL_INNER";
const DUMMY: &str = "owlyd0";

fn ip(args: &[&str]) {
    let ok = Command::new("ip").args(args).status().unwrap().success();
    assert!(ok, "ip {args:?}");
}

// spec: NET-1
#[test]
fn links_and_addresses_change_in_a_namespace() {
    if std::env::var_os(INNER).is_none() {
        let status = Command::new("unshare")
            .args(["--user", "--map-root-user", "--net", "--mount", "--"])
            .args([
                "sh",
                "-ec",
                "mount -t sysfs sysfs /sys && exec \"$0\" \"$@\"",
            ])
            .arg(std::env::current_exe().unwrap())
            .args(["links_and_addresses_change_in_a_namespace", "--exact"])
            .env(INNER, "1")
            .status()
            .expect("running unshare (util-linux)");
        assert!(
            status.success(),
            "failed in its namespace; if unshare itself failed, unprivileged user namespaces may be disabled"
        );
        return;
    }

    ip(&["link", "add", DUMMY, "type", "dummy"]);
    let ops = System;
    let index = ops.index(DUMMY).unwrap();
    let mac: MacAddr = "aa:bb:cc:dd:ee:01".parse().unwrap();
    let mut rtnl = Rtnl::open().unwrap();

    // AWDL-2: the station's MAC on the interface.
    rtnl.set_mac(index, mac).unwrap();
    assert_eq!(ops.mac(DUMMY), Some(mac));
    rtnl.set_mtu(index, 1450).unwrap();
    let mtu = std::fs::read_to_string(format!("/sys/class/net/{DUMMY}/mtu")).unwrap();
    assert_eq!(mtu.trim(), "1450");

    ops.set_up(DUMMY, true).unwrap();
    let flags = std::fs::read_to_string(format!("/sys/class/net/{DUMMY}/flags")).unwrap();
    let flags = u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16).unwrap();
    assert_eq!(flags & 1, 1, "up");

    // NET-1, without DAD: the address added in place of the one the kernel
    // made, if it made one, is usable at once.
    let addr = mac.link_local();
    let _ = rtnl.del_addr(index, addr);
    rtnl.add_link_local_nodad(index, addr).unwrap();
    assert_eq!(sys::link_local(DUMMY), Some((addr, false)));
    rtnl.del_addr(index, addr).unwrap();
    assert_eq!(sys::link_local(DUMMY), None);

    // NET-2: a peer's neighbour entry, from its AWDL address.
    let peer: MacAddr = "86:08:04:8a:2b:f4".parse().unwrap();
    rtnl.add_neighbour(index, peer.link_local(), peer).unwrap();
    rtnl.add_neighbour(index, peer.link_local(), peer).unwrap();
    let neigh = |_: ()| {
        let out = Command::new("ip")
            .args(["-6", "neigh", "show", "dev", DUMMY])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    let shown = neigh(());
    assert!(
        shown.contains("fe80::8408:4ff:fe8a:2bf4 lladdr 86:08:04:8a:2b:f4 PERMANENT"),
        "{shown}"
    );
    rtnl.del_neighbour(index, peer.link_local()).unwrap();
    assert!(!neigh(()).contains("fe80::8408"), "removed");

    ops.set_up(DUMMY, false).unwrap();
    // Errors come back as the kernel's errno.
    let e = rtnl.set_up(999_999, true).unwrap_err();
    assert_eq!(e.raw_os_error(), Some(nix::libc::ENODEV));
}
