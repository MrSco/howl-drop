#!/bin/sh
# Set up HowlDrop under Docker, without Home Assistant OS (README.md here).
#
#   sh install.sh [DIR]        DIR defaults to ~/howldrop
#
# Puts compose.yaml, data/options.json (only if there is none yet) and .env
# (the image for this machine's architecture) in DIR, then pulls the image
# and starts it. Run from a checkout, it copies this folder's files; on its
# own, it downloads them from the repository. Running it again updates the
# compose file and the image and keeps your options.
set -eu

DIR=${1:-$HOME/howldrop}
REPO=https://raw.githubusercontent.com/MrSco/howl-drop/main/docker
HERE=$(cd "$(dirname "$0")" 2>/dev/null && pwd || echo "")

case $(uname -m) in
    aarch64|arm64) ARCH=aarch64 ;;
    x86_64|amd64) ARCH=amd64 ;;
    *)
        echo "HowlDrop's image is for 64-bit ARM (aarch64) and x86-64; this machine is $(uname -m)."
        echo "On a Raspberry Pi, install the 64-bit Raspberry Pi OS."
        exit 1
        ;;
esac
if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is not installed. https://docs.docker.com/engine/install/ (on Raspberry Pi OS: the Debian steps)."
    exit 1
fi
if ! docker compose version >/dev/null 2>&1; then
    echo "The Docker Compose plugin is missing (the docker-compose-plugin package)."
    exit 1
fi
if ! command -v nmcli >/dev/null 2>&1; then
    echo "Note: NetworkManager (nmcli) is not on this machine. HowlDrop takes the Wi-Fi card from"
    echo "NetworkManager while it runs; without it, keep anything else (dhcpcd, wpa_supplicant) off"
    echo "the card yourself. README.md says how."
fi

get() {
    if [ -n "$HERE" ] && [ -f "$HERE/$1" ]; then
        cp "$HERE/$1" "$2"
    else
        curl -fsSL "$REPO/$1" -o "$2"
    fi
}

mkdir -p "$DIR/data" "$DIR/howldrop"
get compose.yaml "$DIR/compose.yaml"
if [ ! -e "$DIR/data/options.json" ]; then
    get options.json "$DIR/data/options.json"
fi
printf 'HOWLDROP_ARCH=%s\n' "$ARCH" > "$DIR/.env"

cd "$DIR"
docker compose pull
docker compose up -d
echo
echo "HowlDrop is starting in $DIR. Follow it with: cd $DIR && docker compose logs -f"
echo "Files land in $DIR/howldrop. Settings: $DIR/data/options.json, then: docker compose restart"
