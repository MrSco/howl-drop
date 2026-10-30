# The HowlDrop add-on. Home Assistant OS stays installed.
# Built from the folder tools/stage-addon.sh assembles: howldrop-agent and
# Owly's crates come from this repo (rust/), so what is edited here ships.
# It ships no Wi-Fi driver: the host kernel drives the card.
# CI builds it for each release as ghcr.io/mrsco/howldrop-{amd64,aarch64};
# a local add-on builds it on the Home Assistant machine.

# The agent: HowlDrop's radio, receiver and sender on Owly's crates
# (rust/, the workspace stage-addon.sh copies in). Built alone, so the
# image carries no compiler.
FROM rust:1.88-slim-bookworm AS agent
WORKDIR /src
COPY rust/ /src/
RUN cargo build --release --locked -p howldrop-agent \
    && install -m 755 target/release/howldrop-agent /usr/local/bin/howldrop-agent

# howldrop-bridge, the forwarder and relay to a PC running howldrop.exe.
FROM debian:bookworm-slim AS bridge
RUN apt-get update && apt-get install -y --no-install-recommends cmake g++ make \
    && rm -rf /var/lib/apt/lists/*
COPY howldrop/ /src/howldrop/
RUN cmake -S /src/howldrop -B /tmp/howldrop-build -DCMAKE_BUILD_TYPE=Release \
    && cmake --build /tmp/howldrop-build --target howldrop-bridge -j"$(nproc)" \
    && install -m 755 /tmp/howldrop-build/howldrop-bridge /usr/local/bin/howldrop-bridge

FROM debian:bookworm-slim

# Set by Supervisor for a local build, and by CI.
ARG BUILD_ARCH=amd64
ARG BUILD_VERSION=dev
LABEL io.hass.type="addon" \
      io.hass.name="HowlDrop" \
      io.hass.arch="${BUILD_ARCH}" \
      io.hass.version="${BUILD_VERSION}" \
      org.opencontainers.image.source="https://github.com/MrSco/howl-drop" \
      org.opencontainers.image.description="AirDrop from iPhones to Home Assistant OS"

ENV DEBIAN_FRONTEND=noninteractive

# iw and iproute2 for the radio (owly-radio runs ip), hostapd for cards with
# no ad-hoc mode, network-manager for nmcli (the host's NetworkManager lets
# go of the card), openssl for Contacts Only, kmod and wireless-regdb for
# WSL, Python for delivery and the panel, tcpdump for debug_capture,
# util-linux for nsenter.
RUN apt-get update && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        hostapd \
        iproute2 \
        iw \
        kmod \
        network-manager \
        openssl \
        procps \
        python3 \
        python3-pip \
        tcpdump \
        util-linux \
        wireless-regdb \
    && rm -rf /var/lib/apt/lists/*

# For howldrop.exe's WSL distro only (wsl-run.sh): there WSL's kernel drives
# the stick and finds no firmware of its own. The add-on never loads these;
# Home Assistant OS has its own. One file per stick in howldrop.exe's table
# (wsl_text.hpp kSticks), laid out as its driver asks for it: the MT7612U's
# (mt76x2u) and the RTL8852BU's (rtw89_8852bu). From one linux-firmware
# commit, and checked by hash.
ARG LINUX_FIRMWARE=https://gitlab.com/kernel-firmware/linux-firmware/-/raw/fed51005f935bab093a4377d0f23f5c7ede81870
RUN mkdir -p /usr/lib/howldrop/firmware/rtw89 && cd /usr/lib/howldrop/firmware \
    && python3 -c "import sys, urllib.request; [urllib.request.urlretrieve(sys.argv[1] + '/' + src, dst) for src, dst in zip(sys.argv[2::2], sys.argv[3::2])]" \
        "$LINUX_FIRMWARE" \
        mediatek/mt7662.bin mt7662.bin \
        mediatek/mt7662_rom_patch.bin mt7662_rom_patch.bin \
        rtw89/rtw8852b_fw-1.bin rtw89/rtw8852b_fw-1.bin \
    && printf '%s\n' \
        'f7e52492f58088cae50e51a54cca68e4abc5b74f7d0b6b731dbb4c04465a94b6  mt7662.bin' \
        '6f0e871268f6e4d99196d90d89bcc09fe493d010366260a2cfcaa5dd66095f8c  mt7662_rom_patch.bin' \
        '5b68415e3bfe72715d63a70703d4471b04b5475b8ff69cfdc8cdf48233cd5d3a  rtw89/rtw8852b_fw-1.bin' \
        | sha256sum -c - \
    && test -f /lib/firmware/regulatory.db-upstream && test -f /lib/firmware/regulatory.db.p7s-upstream

COPY --from=agent /usr/local/bin/howldrop-agent /usr/local/bin/howldrop-agent
COPY --from=bridge /usr/local/bin/howldrop-bridge /usr/local/bin/howldrop-bridge

COPY deliver.py /deliver.py
COPY announce.py /announce.py
COPY panel.py /panel.py
COPY panel/ /panel/
# The panel's thumbnails, and HEIC photos shown as JPEG. From PyPI: pillow-heif
# carries a current libheif, and bookworm's libheif 1.15 cannot read an
# edited iPhone photo (an HDR gain map); it needs a newer Pillow than
# bookworm's too. Checked by a round trip.
RUN pip3 install --no-cache-dir --break-system-packages pillow==12.3.0 pillow-heif==1.1.1 \
    && python3 -c "import pillow_heif; from PIL import Image; pillow_heif.register_heif_opener(); \
Image.new('RGB', (64, 48), (200, 30, 30)).save('/tmp/t.heic'); \
assert Image.open('/tmp/t.heic').convert('RGB').getpixel((5, 5))[0] > 150" \
    && rm /tmp/t.heic
COPY icon.png /panel/icon.png
COPY run.sh /run.sh
COPY wsl-run.sh /wsl-run.sh
RUN chmod 755 /run.sh /wsl-run.sh

ENTRYPOINT ["/bin/bash", "/run.sh"]
