#!/usr/bin/env bash
# Flash + monitor wrapper. Sources wifi.env (gitignored) for build-time secrets,
# then runs cargo espflash. Forward any extra args, e.g. `./flash.sh --port /dev/cu.usbmodem101`.
set -euo pipefail

cd "$(dirname "$0")"

if [[ ! -f wifi.env ]]; then
    echo "wifi.env not found. Create one with WIFI_SSID/WIFI_PASSWORD exports."
    exit 1
fi

# shellcheck disable=SC1091
source wifi.env

# Activate the Xtensa Rust toolchain if not already on PATH.
if ! rustc -Vv 2>/dev/null | grep -q xtensa; then
    if [[ -f "$HOME/export-esp.sh" ]]; then
        # shellcheck disable=SC1091
        source "$HOME/export-esp.sh"
    fi
fi

# The firmware lives in the `firmware` member crate. espflash reads
# espflash.toml from the current directory, and the partition relative path
# baked into Cargo metadata is resolved from the member's manifest dir, so
# run cargo from inside crates/firmware/.
cd crates/firmware
exec cargo espflash flash --release --monitor "$@"
