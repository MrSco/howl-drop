# Local Home Assistant add-on. Home Assistant OS stays installed.
# The image builds the released omdrop-owl tree (IBSS exclusive for iwlwifi).
# It does not check out pull request 13, and it does not ship Intel firmware.
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
        procps \
        python3 \
        python3-dbus \
        python3-gi \
        python3-libarchive-c \
    && rm -rf /var/lib/apt/lists/*

# Released main after the AUR publish. The iwlwifi line is exclusive IBSS.
# Pull request 13 (USB MT7612U active-monitor) is not this commit.
ARG OMDROP_OWL_COMMIT=8c38540267641ba2dcdf16b0ae115ac71f11be94
ARG OMDROP_PLUGIN_COMMIT=b2f9a537b150c26402c5daad8125fb7e61c214ec

RUN git clone https://github.com/t4t5/omdrop-owl.git /tmp/omdrop-owl \
    && git -C /tmp/omdrop-owl checkout "$OMDROP_OWL_COMMIT" \
    && awk ' \
        $1 == "iwlwifi" && $2 == "tested" && $3 == "exclusive" { ok = 1 } \
        $1 == "mt76x2u" || $7 == "active-monitor" { bad = 1 } \
        END { if (!ok || bad) exit 1 } \
      ' /tmp/omdrop-owl/userspace/owl-profiles \
    && grep -q '00:25:00:ff:94:73' /tmp/omdrop-owl/userspace/omdrop-discoverable \
    && /tmp/omdrop-owl/tools/build-owl.sh \
    && /tmp/omdrop-owl/tools/stage.sh \
    && mkdir -p /usr/lib/omdrop \
    && cp -a /tmp/omdrop-owl/build/lib/. /usr/lib/omdrop/ \
    && rm -rf /tmp/omdrop-owl /usr/lib/omdrop/__pycache__

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

COPY owldrop/ /src/owldrop/
RUN cmake -S /src/owldrop -B /tmp/owldrop-build -DCMAKE_BUILD_TYPE=Release \
    && cmake --build /tmp/owldrop-build --target owldrop-bridge -j"$(nproc)" \
    && install -m 755 /tmp/owldrop-build/owldrop-bridge /usr/local/bin/owldrop-bridge \
    && rm -rf /tmp/owldrop-build

COPY run.sh /run.sh
RUN chmod 755 /run.sh

ENTRYPOINT ["/bin/bash", "/run.sh"]
