# owly for Fedora. packaging/package.sh rpm builds it: it stages the files
# with packaging/install.sh and defines owly_version, owly_release,
# owly_stage (the staged tree) and owly_files (the %files list), so this
# only packs them.
Name: owly
Version: %{owly_version}
Release: %{owly_release}
Summary: AirDrop's Wi-Fi radio on Linux
License: MIT
URL: https://github.com/t4t5/owly
# The AUR's depends, by Fedora's names: see packaging/arch/owly/PKGBUILD for
# what each is for. rpmbuild adds the libraries the binaries link.
Requires: dbus systemd polkit NetworkManager iproute iputils util-linux-core
Recommends: hostapd
Suggests: bluez

# The binaries are stripped already, and have no debug info to split off.
%global debug_package %{nil}
# Its date would come from %changelog, which the release notes stand in for.
%global source_date_epoch_from_changelog 0

%description
AWDL in userspace, for Wi-Fi cards without Apple's Broadcom firmware:
owlyd, the system service that leases the radio out over D-Bus, and the
owly CLI: doctor, test-card, send and receive.

%install
cp -a %{owly_stage}/. %{buildroot}/

%files -f %{owly_files}

# The scriptlets are packaging/arch/owly/owly.install's. systemd's own file
# triggers reload units, and D-Bus notices its new files itself.
%post
# NetworkManager reads conf.d only at startup or on a reload, so keeping
# Owly's interfaces unmanaged isn't in force until something rereads it.
# `reload conf` touches no connection.
if systemctl is-active --quiet NetworkManager 2>/dev/null; then
  nmcli general reload conf >/dev/null 2>&1 \
    || echo "  Could not reload NetworkManager's configuration. Run: sudo nmcli general reload conf"
fi
if [ "$1" -eq 1 ]; then
  cat <<'MSG'
  Owly's radio service, owlyd, starts when something asks for the radio.
  `owly doctor` says whether this Wi-Fi card can do AirDrop and what, if
  anything, is in the way.
MSG
# A running owlyd keeps the old binary until it next starts. Restart it now
# only when it has no session, so an upgrade doesn't end a window. systemd's
# file triggers reload units only after the transaction, so reload first:
# otherwise the restart runs the old unit's ExecStart, which may name the
# old package's owlyd (it was in /usr/lib/owly before /usr/libexec).
elif [ "$(busctl get-property io.github.t4t5.Owly /io/github/t4t5/Owly \
    io.github.t4t5.Owly.Radio1 State 2>/dev/null)" = 's "idle"' ]; then
  systemctl daemon-reload || :
  systemctl try-restart owlyd.service || :
fi

%preun
# Gives Wi-Fi back if a session is up.
if [ "$1" -eq 0 ]; then
  systemctl stop owlyd.service 2>/dev/null || :
fi
