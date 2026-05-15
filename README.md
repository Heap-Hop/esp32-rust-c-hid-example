# esp32-rust-c-hid-example

End-to-end example of driving a [TinyUSB](https://github.com/hathach/tinyusb) HID
device (keyboard + mouse + media keys) on an ESP32-S3 over a plain-text UDP
protocol. Any client that can open a UDP socket — including `nc` — can type on
the host computer, move the mouse, and send media keys.

Generated from [esp32-rust-c-template](https://github.com/Heap-Hop/esp32-rust-c-template).
It demonstrates how to layer a hand-written, fully safe Rust API on top of a
non-trivial remote ESP-IDF component (`espressif/esp_tinyusb`).

The companion phone / desktop client is
[Heap-Hop/local-hid](https://github.com/Heap-Hop/local-hid) — a Flutter app
exposing a touchpad, virtual keyboard, and media keys that speak this protocol
over UDP.

## Hardware

Any ESP32-S3 board with a dedicated **native USB port** (the pins labelled
`D+`/`D-` or `USB` — GPIO19/GPIO20). Plug that port into the computer that you
want the ESP32-S3 to act as a HID device for.

Notes:

- On boards with two USB-C connectors, one is the USB-Serial/JTAG port (used
  for flashing and `cargo espflash`) and the other is the native USB peripheral
  (used by the HID device). Use the right one for the right job.
- On single-port boards (e.g. AtomS3R) the one port is switched between modes.
  Flash in bootloader mode, then press reset — when the firmware runs the same
  port acts as the HID device. `--monitor` is not available in HID mode.

## Setup

Install the Rust ESP toolchain once (see the template README for full
instructions). Briefly:

```bash
cargo install espup ldproxy espflash cargo-espflash
espup install
. $HOME/export-esp.sh
```

ESP-IDF `v5.5.3` is downloaded automatically on the first build.

## Build-time configuration

Three environment variables are read by `build.rs` and baked into the firmware:

| Variable         | Required | Default | Meaning                                    |
|------------------|----------|---------|--------------------------------------------|
| `WIFI_SSID`      | yes      | —       | Network SSID (2.4 GHz, WPA2-Personal)      |
| `WIFI_PASSWORD`  | no       | empty   | Leave empty for open networks              |
| `UDP_PORT`       | no       | `9000`  | UDP port the HID command server listens on |

Export them in your shell before building:

```bash
export WIFI_SSID="my-wifi"
export WIFI_PASSWORD="my-password"
# export UDP_PORT=9000
```

The build is cached per value — changing any of them triggers a rebuild.

## Build and flash

```bash
cargo espflash flash --release --monitor
```

The serial monitor logs the assigned IP once Wi-Fi is up:

```
I (...) esp32_rust_c_hid_example: Wi-Fi connected: IpInfo { ip: 192.168.1.42, ... }
I (...) esp32_rust_c_hid_example: UDP HID command server listening on 0.0.0.0:9000
```

## Command protocol

Send one command per line over UDP. Tokens are separated by whitespace, case
is ignored for verbs and key names, and each command replies with a single
line starting with `ok` or `err`.

```
key  <name>                   tap a key
k    <name>                   short alias for `key`

mouse <dx> <dy>               relative mouse movement, -127..127 per axis
m     <dx> <dy>

click [left|right|middle]     mouse button click, default left
c     [left|right|middle]

media <name>                  consumer-control key tap
md    <name>

help                          list commands
```

### Supported key names

- Single character: letters `a`..`z` (case-insensitive), digits `0`..`9`.
- Named:  `enter` / `return`, `esc` / `escape`, `backspace`, `tab`, `space`,
  `left`, `right`, `up`, `down`.

### Supported media names

`play`, `pause`, `playpause`, `next`, `prev` / `previous`, `stop`, `mute`,
`volup`, `voldown`.

## Testing with `nc`

Open a UDP session and type commands interactively (`-u` for UDP):

```
$ nc -u 192.168.1.42 9000
k a
ok key a
k enter
ok key enter
mouse 50 -20
ok mouse 50 -20
click right
ok click right
md volup
ok media volup
md playpause
ok media playpause
```

Or stream a prepared script:

```bash
printf 'k h\nk i\nk enter\n' | nc -u -w1 192.168.1.42 9000
```

The ESP32-S3's native USB port must be connected to a host computer — that is
the computer whose keyboard / mouse the commands control.

## How it works

### C side: `components/tinyusb_bridge/`

A thin C component that:

- Declares the HID report descriptor (boot keyboard + mouse + consumer control
  on three distinct report IDs). These are built with TinyUSB macros that are
  awkward to expand in Rust, so that portion stays in C.
- Implements `tud_hid_descriptor_report_cb` (required by TinyUSB at
  enumeration time).
- Exposes a small bindgen-friendly C API (`tinyusb_bridge_*`) over the
  otherwise-complex TinyUSB submission calls, including retry loops that wait
  for the HID endpoint to become ready.

### Rust side

`src/tinyusb_hid.rs` wraps the C API:

- `mod ffi { unsafe extern "C" { ... } }` contains every `extern "C"` — this
  is the only `unsafe` surface in the binary.
- Public functions return `Result<(), EspError>`, use
  `esp_idf_sys::esp!(...)` to convert status codes, and serialise calls via a
  single process-wide `Mutex`.
- The TinyUSB weak-symbol callbacks `tud_hid_get_report_cb` and
  `tud_hid_set_report_cb` are implemented in Rust with `#[no_mangle] extern
  "C"`. No extra C stub is needed for them.

`src/protocol.rs` parses the text commands into a strongly typed `Command`
enum. It has unit tests — run them on the host with `cargo test --target
$(rustc -vV | grep host | cut -d' ' -f2) --lib` (the build target is
Xtensa, tests need a host target).

`src/main.rs` wires everything together — link patches, logger, TinyUSB init,
Wi-Fi, UDP socket, recv/dispatch loop. It contains **zero** `unsafe` blocks.

## Project layout

```
.cargo/config.toml          target chip, linker, ESP-IDF version, BINDGEN flags
Cargo.toml                  Rust deps + local + remote extra_components
build.rs                    embuild output + WIFI_SSID/WIFI_PASSWORD/UDP_PORT
sdkconfig.defaults          stack sizes, TinyUSB HID count, partition table
partitions.csv              custom 3 MB factory app partition
espflash.toml               points espflash at partitions.csv

src/
  main.rs                   application entry — no unsafe
  tinyusb_hid.rs            FFI wrapper (the ONLY unsafe in the project)
  protocol.rs               text command parser + HID usage tables

components/
  tinyusb_bridge/
    CMakeLists.txt          requires espressif__esp_tinyusb
    include/tinyusb_bridge.h
    tinyusb_bridge.c        HID descriptor + submission helpers
```

## Troubleshooting

**Device never appears as a HID on the host.** Check the USB cable — some are
charge-only. Confirm the ESP32-S3 native USB port is plugged in, not the
USB-Serial port. Linux: `dmesg | tail` and `lsusb` should show a new device
with VID/PID from TinyUSB's defaults.

**`err mouse: ESP_FAIL` (or similar) on every command.** The HID interface is
not mounted yet. Wait a few seconds after boot, or re-plug the USB cable.

**`WIFI_SSID must be set before building firmware` at compile time.** Export
the env var in your shell and re-run `cargo build`.

**C source changes in `components/` not picked up.** Cargo tracks build script
inputs, not C files directly. Force a re-run with:

```bash
rm target/xtensa-esp32s3-espidf/release/build/esp-idf-sys-*/output
cargo build -r
```

**Build fails with `partitions.csv ... missing and no known rule to make it`.**
`CONFIG_PARTITION_TABLE_CUSTOM_FILENAME` is resolved relative to the ESP-IDF
build output directory, not the project root, so the CSV has to be copied in
by the Rust build. This template already does that via the following lines in
`.cargo/config.toml`:

```toml
[env]
ESP_IDF_GLOB_FIRMWARE_BASE = { value = "", relative = true }
ESP_IDF_GLOB_FIRMWARE_PARTITIONS = "partitions.csv"
```

If you rename the CSV or move it, update the second line to match.
