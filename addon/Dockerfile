# The HowlDrop add-on. Home Assistant OS stays installed.
# Built from the folder tools/stage-addon.sh assembles: the radio stack comes
# from this repo (omdrop-owl/), not a clone, so helper edits ship with it.
# It ships no Wi-Fi driver or firmware: the host kernel drives the card.
# CI builds it for each release as ghcr.io/mrsco/howldrop-{amd64,aarch64};
# a local add-on builds it on the Home Assistant machine.
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

RUN apt-get update && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        cmake \
        dbus \
        ethtool \
        g++ \
        git \
        gir1.2-glib-2.0 \
        iproute2 \
        iw \
        kmod \
        libev-dev \
        libnl-3-dev \
        libnl-genl-3-dev \
        libnl-route-3-dev \
        libpcap-dev \
        make \
        network-manager \
        openssl \
        pkg-config \
        tcpdump \
        procps \
        python3 \
        python3-dbus \
        python3-gi \
        python3-pip \
        python3-libarchive-c \
        wireless-regdb \
    && rm -rf /var/lib/apt/lists/*

# For howldrop.exe's WSL distro only (wsl-run.sh): there WSL's kernel drives
# the stick and finds no firmware of its own. The add-on never loads these;
# Home Assistant OS has its own. Pinned by hash; linux-firmware's mt7662 files
# date from 2015.
ARG LINUX_FIRMWARE=https://gitlab.com/kernel-firmware/linux-firmware/-/raw/main/mediatek
RUN mkdir -p /usr/lib/howldrop/firmware && cd /usr/lib/howldrop/firmware \
    && python3 -c "import sys, urllib.request; [urllib.request.urlretrieve(sys.argv[1] + '/' + n, n) for n in sys.argv[2:]]" \
        "$LINUX_FIRMWARE" mt7662.bin mt7662_rom_patch.bin \
    && printf '%s\n' \
        'f7e52492f58088cae50e51a54cca68e4abc5b74f7d0b6b731dbb4c04465a94b6  mt7662.bin' \
        '6f0e871268f6e4d99196d90d89bcc09fe493d010366260a2cfcaa5dd66095f8c  mt7662_rom_patch.bin' \
        | sha256sum -c - \
    && test -f /lib/firmware/regulatory.db-upstream && test -f /lib/firmware/regulatory.db.p7s-upstream

# 0.16.3. Since 0.16.2 the receiver stores an upload whose closing chunk
# never comes (upstream pull requests 41 and 47).
ARG OMDROP_PLUGIN_COMMIT=f612996ac5a8321438ca83fa0d84a8a678b6e427

# OWL first, in its own layer: it is the slow compile, and it depends only on
# pins and patches/. Scripts run through bash because a copy over the Samba
# share loses their executable bit.
COPY omdrop-owl/pins /src/omdrop-owl/pins
COPY omdrop-owl/patches/ /src/omdrop-owl/patches/
COPY omdrop-owl/tools/build-owl.sh /src/omdrop-owl/tools/build-owl.sh
RUN bash /src/omdrop-owl/tools/build-owl.sh

# contacts.py looks for the Apple root two directories above itself, in
# share/certs. The receiver is /opt/omdrop-receiver/contacts.py, so the
# certificate has to be /opt/share/certs/apple_root_ca.pem. The issuing
# intermediates stay beside contacts.py, in apple-ca/.
RUN git clone https://github.com/netmojo-systems/omdrop-plugin.git /tmp/omdrop-plugin \
    && git -C /tmp/omdrop-plugin checkout "$OMDROP_PLUGIN_COMMIT" \
    && mkdir -p /opt/omdrop-receiver /opt/share \
    && cp -a /tmp/omdrop-plugin/bin/. /opt/omdrop-receiver/ \
    && cp -a /tmp/omdrop-plugin/share/. /opt/share/ \
    && test -s /opt/share/certs/apple_root_ca.pem \
    && test -s /opt/omdrop-receiver/apple-ca/apple-aai-ca.pem \
    && rm -rf /tmp/omdrop-plugin

# A stalled upload logs where its unread bytes are, and the log level
# follows the verbose option (patch_receiver.py). For howldrop.exe in WSL
# only: Accept / Decline on the PC and the upload's progress (howl_ask.py).
COPY patch_receiver.py /tmp/patch_receiver.py
COPY howl_ask.py /opt/omdrop-receiver/howl_ask.py
RUN python3 /tmp/patch_receiver.py /opt/omdrop-receiver/airdrop-serve.py \
    && rm /tmp/patch_receiver.py

# The helper and its profiles, with omdrop-awdl's radio-independent tools,
# laid out as /usr/lib/omdrop.
COPY omdrop-owl/tools/stage.sh /src/omdrop-owl/tools/stage.sh
COPY omdrop-owl/userspace/ /src/omdrop-owl/userspace/
RUN bash /src/omdrop-owl/tools/stage.sh \
    && mkdir -p /usr/lib/omdrop \
    && cp -a /src/omdrop-owl/build/lib/. /usr/lib/omdrop/ \
    && rm -rf /src/omdrop-owl/build /usr/lib/omdrop/__pycache__

COPY howldrop/ /src/howldrop/
RUN cmake -S /src/howldrop -B /tmp/howldrop-build -DCMAKE_BUILD_TYPE=Release \
    && cmake --build /tmp/howldrop-build --target howldrop-bridge -j"$(nproc)" \
    && install -m 755 /tmp/howldrop-build/howldrop-bridge /usr/local/bin/howldrop-bridge \
    && rm -rf /tmp/howldrop-build

COPY deliver.py /deliver.py
COPY outbox.py /outbox.py
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
COPY senders.py /senders.py
COPY run.sh /run.sh
COPY wsl-run.sh /wsl-run.sh
RUN chmod 755 /run.sh /wsl-run.sh

ENTRYPOINT ["/bin/bash", "/run.sh"]
