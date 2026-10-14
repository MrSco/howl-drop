# Local Home Assistant add-on. Home Assistant OS stays installed.
# Built from the folder tools/stage-addon.sh assembles: the radio stack comes
# from this repo (omdrop-owl/), not a clone, so helper edits ship with it.
# It does not ship Intel firmware.
FROM debian:bookworm-slim

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
        python3-libarchive-c \
    && rm -rf /var/lib/apt/lists/*

ARG OMDROP_PLUGIN_COMMIT=b2f9a537b150c26402c5daad8125fb7e61c214ec

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
RUN git clone https://github.com/brentkearney/omdrop-plugin.git /tmp/omdrop-plugin \
    && git -C /tmp/omdrop-plugin checkout "$OMDROP_PLUGIN_COMMIT" \
    && mkdir -p /opt/omdrop-receiver /opt/share \
    && cp -a /tmp/omdrop-plugin/bin/. /opt/omdrop-receiver/ \
    && cp -a /tmp/omdrop-plugin/share/. /opt/share/ \
    && test -s /opt/share/certs/apple_root_ca.pem \
    && test -s /opt/omdrop-receiver/apple-ca/apple-aai-ca.pem \
    && rm -rf /tmp/omdrop-plugin

# A stalled upload logs where its unread bytes are (patch_receiver.py).
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

COPY owldrop/ /src/owldrop/
RUN cmake -S /src/owldrop -B /tmp/owldrop-build -DCMAKE_BUILD_TYPE=Release \
    && cmake --build /tmp/owldrop-build --target owldrop-bridge -j"$(nproc)" \
    && install -m 755 /tmp/owldrop-build/owldrop-bridge /usr/local/bin/owldrop-bridge \
    && rm -rf /tmp/owldrop-build

COPY deliver.py /deliver.py
COPY senders.py /senders.py
COPY run.sh /run.sh
RUN chmod 755 /run.sh

ENTRYPOINT ["/bin/bash", "/run.sh"]
