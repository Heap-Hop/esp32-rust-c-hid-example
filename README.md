# esp32-rust-c-hid-example

End-to-end example of driving a [TinyUSB](https://github.com/hathach/tinyusb) HID
device (keyboard + mouse + media keys) on an ESP32-S3. The firmware accepts a
compact binary command protocol over UDP and replays each command as a USB HID
report on the host computer it is plugged into.

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

Send one command per UDP datagram. The wire format is a fixed binary header
followed by an opcode-specific payload — see [`src/protocol.rs`](src/protocol.rs)
for the canonical definition. All datagrams are fire-and-forget except `PING`,
which the firmware answers with a matching `PONG`.

```
header   [0]  magic    0x48 ('H')        — quick filter for stray traffic
         [1]  version  0x01              — protocol version
         [2]  opcode

op    name           payload                          notes
0x01  KEY_TAP        u8 modifier, u8 keycode          press + small hold + release
0x02  KEY_DOWN       u8 modifier, u8 keycode          press, no release (for chording)
0x03  KEY_UP         (none)                           release whatever is held
0x10  MOUSE_MOVE     i8 dx, i8 dy, i8 wheel           relative; wheel reserved (this fw ignores)
0x11  MOUSE_CLICK    u8 button_mask                   down + small hold + up
0x12  MOUSE_BUTTONS  u8 button_mask                   raw button state (for drags)
0x20  MEDIA_TAP      u16 le usage_code                consumer-control key tap
0xf0  PING           u32 le seq                       app → fw
0xf1  PONG           u32 le seq                       fw → app reply (magic is lowercase 'h')
```

`modifier` and `keycode` follow HID Usage Page 0x07 (Keyboard); `button_mask`
is the standard HID Mouse mask (bit 0 = left, bit 1 = right, bit 2 = middle);
`usage_code` is HID Usage Page 0x0c (Consumer Control).

The minimum datagram is 3 bytes (KEY_UP) and the maximum used by v1 is 7
bytes (PING). The 256-byte recv buffer leaves room for future opcodes.

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

`src/protocol.rs` parses incoming datagrams into a strongly typed `Command`
enum, no allocation, no UTF-8 work. It ships with unit tests covering header
validation and each opcode's payload layout.

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
