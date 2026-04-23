# esp32-s3-hid-example

> Generated from [esp32-rust-c-template](https://github.com/Heap-Hop/esp32-rust-c-template). To start a new project from the same template:
> ```bash
> cargo install cargo-generate
> cargo generate --git https://github.com/Heap-Hop/esp32-rust-c-template --name my-firmware
> ```

Minimal starting point for ESP32-S3 firmware written in Rust with C components linked via hand-written FFI.

The split this template is built around:

- **Rust** handles application logic: networking, protocols, state machines, error handling.
- **C** handles anything that is easier or only available as a C library: drivers, USB stacks, vendor SDKs, remote ESP-IDF components.

The FFI boundary is declared by hand in `src/main.rs`. For small, stable C APIs this is simpler than a bindgen step and keeps the `unsafe` surface area reviewable.

## Prerequisites

Install the Rust ESP toolchain once:

```bash
cargo install espup ldproxy espflash cargo-espflash
espup install
```

If you have [cargo-binstall](https://github.com/cargo-bins/cargo-binstall), use it instead to download pre-built binaries and skip compilation:

```bash
cargo binstall espup ldproxy espflash cargo-espflash
espup install
```

`espup install` writes `$HOME/export-esp.sh` and handles the Xtensa toolchain. ESP-IDF itself is downloaded automatically on the first build (see below).

## Per-shell setup

Source the Rust ESP environment before every build session:

```bash
. $HOME/export-esp.sh
```

ESP-IDF does **not** need to be sourced manually — this template sets `ESP_IDF_TOOLS_INSTALL_DIR = "global"` in `.cargo/config.toml`, so the first `cargo build` downloads ESP-IDF into `~/.espressif/` automatically and reuses it for all subsequent builds.

The pinned version is `v5.5.3`. To use a different version, edit `.cargo/config.toml`:

```toml
ESP_IDF_VERSION = "v5.5.3"
```

## Build and flash

```bash
cargo build -r
cargo espflash flash --release --monitor
```

`cargo espflash flash --release --monitor` rebuilds the release image itself before flashing, so the explicit `cargo build -r` step is only needed if you want to compile without flashing.

## Project layout

```
.cargo/config.toml          target chip, linker, build-std, ESP-IDF version
Cargo.toml                  Rust deps + component_dirs declaration
build.rs                    single embuild call (required)
sdkconfig.defaults          stack sizes (Rust needs more than C defaults)
rust-toolchain.toml         pins the esp Xtensa toolchain channel
src/
  main.rs                   application entry point — no unsafe
  c_example.rs              safe Rust wrapper around the C component
components/
  c_example/
    CMakeLists.txt          registers the component with ESP-IDF
    include/c_example.h     public API (must be bindgen-safe)
    c_example.c             implementation
```

## How to add a local C component

1. Create `components/<your_component>/` with a `CMakeLists.txt`, a header under `include/`, and a `.c` file.

   Minimal `CMakeLists.txt`:
   ```cmake
   idf_component_register(
       SRCS "your_component.c"
       INCLUDE_DIRS "include"
       REQUIRES some_idf_component   # optional
   )
   ```

2. Keep the public header bindgen-safe: use only `stdint.h` integer types, `bool`, `esp_err_t`, and plain pointers. No C++ features, no complex macros, no non-standard extensions.

3. Create `src/<your_component>.rs` with a private `ffi` block and safe public wrappers:

   ```rust
   use esp_idf_sys::EspError;

   mod ffi {
       use esp_idf_sys::esp_err_t;
       unsafe extern "C" {
           pub fn your_component_init() -> esp_err_t;
           pub fn your_component_do(arg: i32) -> i32;
       }
   }

   pub fn init() -> Result<(), EspError> {
       esp_idf_sys::esp!(unsafe { ffi::your_component_init() })
   }

   pub fn do_thing(arg: i32) -> i32 {
       unsafe { ffi::your_component_do(arg) }
   }
   ```

4. In `src/main.rs`, declare the module and call the safe API — no `unsafe` needed:

   ```rust
   mod your_component;

   // in main():
   your_component::init().expect("init failed");
   let result = your_component::do_thing(42);
   ```

No changes to `Cargo.toml` are needed for local components — `component_dirs = ["components"]` already covers the whole directory.

## How to add a remote component (ESP-IDF Component Registry)

Remote components are downloaded by `idf-component-manager` at build time.

1. Add an entry in `Cargo.toml`:
   ```toml
   [[package.metadata.esp-idf-sys.extra_components]]
   remote_component = { name = "espressif/mdns", version = "^1.2" }
   ```

2. Write a thin C bridge in `components/` that wraps the remote component and exposes a bindgen-safe header. The remote component's own headers often use constructs that conflict with the bindings `esp-idf-sys` already generated; the bridge layer isolates Rust from that.

3. In the bridge `CMakeLists.txt`, reference the downloaded component by its `vendor__name` form:
   ```cmake
   idf_component_register(
       SRCS "mdns_bridge.c"
       INCLUDE_DIRS "include"
       REQUIRES espressif__mdns
   )
   ```

4. Declare the bridge functions in Rust as usual.

For a complete worked example of adding a non-trivial remote component — including bindgen header conflicts, `#[no_mangle]` callbacks in Rust, and a custom partition table — see the [reference project](#reference-a-fully-built-project-on-top-of-this-template) below.

## Troubleshooting: C source changes not picked up

Cargo tracks build script inputs, not C source files directly. If you edit a
`.c` file in `components/` and `cargo build` does not recompile it, force a
rebuild by deleting the esp-idf-sys build script output:

```bash
rm target/xtensa-esp32s3-espidf/release/build/esp-idf-sys-*/output
cargo build -r
```

Cargo will re-run the build script, which re-invokes CMake/Ninja and recompiles
the changed component.

`cargo clean` also works but discards the entire incremental cache (slow first
rebuild). The `output`-deletion approach is surgical and only re-runs the C
build step.

## Targeting a different chip

This template is wired for ESP32-S3. To retarget, edit `.cargo/config.toml`:

| Chip      | `target`                    | `MCU`     |
|-----------|-----------------------------|-----------|
| ESP32     | `xtensa-esp32-espidf`       | `esp32`   |
| ESP32-S2  | `xtensa-esp32s2-espidf`     | `esp32s2` |
| ESP32-C3  | `riscv32imc-esp-espidf`     | `esp32c3` |
| ESP32-C6  | `riscv32imac-esp-espidf`    | `esp32c6` |
| ESP32-H2  | `riscv32imac-esp-espidf`    | `esp32h2` |

The next `cargo build` will automatically download the correct toolchain for the new chip.

## Reference: a fully-built project on top of this template

For a complete end-to-end example that adds Wi-Fi, a TCP command protocol, and
a TinyUSB HID bridge on top of this template, see:

> _(link to reference project — fill in once published)_

That project demonstrates the C-bridge pattern for a non-trivial remote
component (`espressif/esp_tinyusb`), a custom partition table, and
build-time configuration via environment variables.
