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
        ethtool \
        g++ \
        git \
        gir1.2-glib-2.0 \
        iproute2 \
        iw \
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
        python3-pil \
        libheif-examples \
        python3-libarchive-c \
    && rm -rf /var/lib/apt/lists/*

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
# follows the verbose option (patch_receiver.py).
COPY patch_receiver.py /tmp/patch_receiver.py
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
COPY announce.py /announce.py
COPY panel.py /panel.py
COPY panel/ /panel/
# The panel shows HEIC photos as JPEG through heif-convert: check it decodes.
RUN python3 -c "from PIL import Image; Image.new('RGB', (64, 48), (200, 30, 30)).save('/tmp/t.png')" \
    && heif-enc -o /tmp/t.heic /tmp/t.png >/dev/null \
    && heif-convert /tmp/t.heic /tmp/t.jpg >/dev/null \
    && python3 -c "from PIL import Image; assert Image.open('/tmp/t.jpg').size == (64, 48)" \
    && rm /tmp/t.png /tmp/t.heic /tmp/t.jpg
COPY icon.png /panel/icon.png
COPY senders.py /senders.py
COPY run.sh /run.sh
RUN chmod 755 /run.sh

ENTRYPOINT ["/bin/bash", "/run.sh"]
