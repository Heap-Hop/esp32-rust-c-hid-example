//! ESP32-S3 TinyUSB HID over TCP example.
//!
//! At boot:
//!   1. Install the TinyUSB HID bridge so the device enumerates as a keyboard
//!      + mouse on the USB port.
//!   2. Connect to Wi-Fi using credentials baked in at build time.
//!   3. Listen on a TCP port; each client can send newline-delimited text
//!      commands (see `protocol.rs`) and each one is turned into a HID report.
//!
//! Test from any host on the same network:
//!   nc <board-ip> 9000
//!   key a
//!   mouse 20 -5
//!   media volup
//!
//! Neither `main.rs` nor any other file outside `tinyusb_hid.rs` contains an
//! `unsafe` block — the FFI is fully encapsulated there.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{
    AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
};

mod protocol;
mod tinyusb_hid;

use protocol::{Command, MouseButton};

// Baked-in build-time configuration (see build.rs).
const BUILT_WIFI_SSID: &str = env!("WIFI_SSID_BUILT");
const BUILT_WIFI_PASSWORD: &str = env!("WIFI_PASSWORD_BUILT");
const BUILT_TCP_PORT: &str = env!("TCP_PORT_BUILT");

/// Delay between key press and key release for a "tap".
const TAP_HOLD: Duration = Duration::from_millis(12);
/// Cap each command line to avoid unbounded buffering from a misbehaving peer.
const MAX_COMMAND_BYTES: usize = 4 * 1024;

fn main() -> anyhow::Result<()> {
    // Required ESP-IDF setup — always before anything else.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let tcp_port: u16 = BUILT_TCP_PORT
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid TCP_PORT '{BUILT_TCP_PORT}': {e}"))?;
    log::info!(
        "esp32-rust-c-hid-example starting (SSID='{}', TCP port={})",
        BUILT_WIFI_SSID,
        tcp_port,
    );

    tinyusb_hid::init()?;
    log::info!("TinyUSB HID bridge initialised");

    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    // Keep the Wi-Fi guard alive for the whole program.
    let _wifi = connect_wifi(peripherals, sysloop, nvs)?;

    run_tcp_server(tcp_port)
}

fn connect_wifi(
    peripherals: Peripherals,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> anyhow::Result<BlockingWifi<EspWifi<'static>>> {
    let auth_method = if BUILT_WIFI_PASSWORD.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };

    let esp_wifi = EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs))?;
    let mut wifi = BlockingWifi::wrap(esp_wifi, sysloop)?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: BUILT_WIFI_SSID.try_into().map_err(|_| {
            anyhow::anyhow!("SSID '{}' exceeds 32 bytes", BUILT_WIFI_SSID)
        })?,
        password: BUILT_WIFI_PASSWORD.try_into().map_err(|_| {
            anyhow::anyhow!("Wi-Fi password exceeds 64 bytes")
        })?,
        auth_method,
        ..Default::default()
    }))?;

    log::info!("connecting to Wi-Fi...");
    wifi.start()?;
    wifi.connect()?;
    wifi.wait_netif_up()?;
    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
    log::info!("Wi-Fi connected: {:?}", ip_info);

    Ok(wifi)
}

fn run_tcp_server(port: u16) -> anyhow::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    log::info!("TCP HID command server listening on 0.0.0.0:{port}");

    loop {
        let (stream, addr) = listener.accept()?;
        log::info!("client connected: {addr}");
        thread::Builder::new()
            .stack_size(8 * 1024)
            .spawn(move || {
                if let Err(error) = handle_client(stream) {
                    log::warn!("client {addr} disconnected: {error}");
                }
            })?;
    }
}

fn handle_client(mut stream: TcpStream) -> io::Result<()> {
    // Note: we deliberately do NOT call `stream.try_clone()` to split the
    // socket into a separate reader/writer. ESP-IDF's lwIP does not implement
    // `dup(2)`, so `try_clone` fails with ENOSYS ("Function not implemented").
    // Single-byte reads on the shared `stream` are slower in theory but
    // perfectly fine for a human-typed command protocol.
    stream.set_nodelay(true).ok();
    writeln!(&mut stream, "ok hello — send 'help' for commands")?;

    let mut line_buffer = Vec::with_capacity(64);
    loop {
        line_buffer.clear();
        match read_line(&mut stream, &mut line_buffer)? {
            ReadLine::Eof => return Ok(()),
            ReadLine::TooLong => {
                writeln!(&mut stream, "err line too long")?;
                continue;
            }
            ReadLine::Line => {}
        }

        let line = match std::str::from_utf8(&line_buffer) {
            Ok(s) => s.trim(),
            Err(_) => {
                writeln!(&mut stream, "err invalid utf-8")?;
                continue;
            }
        };
        if line.is_empty() {
            continue;
        }

        let reply = handle_line(line);
        writeln!(&mut stream, "{reply}")?;
    }
}

enum ReadLine {
    Line,
    Eof,
    TooLong,
}

/// Read until `\n` or EOF into `buffer` (excluding the newline). `\r` is
/// stripped so clients sending `\r\n` (Windows telnet) still work.
fn read_line(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> io::Result<ReadLine> {
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte)? {
            0 if buffer.is_empty() => return Ok(ReadLine::Eof),
            0 => return Ok(ReadLine::Line),
            _ => match byte[0] {
                b'\n' => return Ok(ReadLine::Line),
                b'\r' => continue,
                byte => {
                    if buffer.len() >= MAX_COMMAND_BYTES {
                        // Drain the rest of the line so we resync to the next
                        // newline, then report the overflow to the caller.
                        drain_until_newline(stream)?;
                        return Ok(ReadLine::TooLong);
                    }
                    buffer.push(byte);
                }
            },
        }
    }
}

fn drain_until_newline(stream: &mut TcpStream) -> io::Result<()> {
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte)? {
            0 => return Ok(()),
            _ if byte[0] == b'\n' => return Ok(()),
            _ => {}
        }
    }
}

/// Parse one line, execute it against the HID bridge, and return a single-line
/// reply (always starting with `ok` or `err`) to send back to the client.
fn handle_line(line: &str) -> String {
    match protocol::parse(line) {
        Command::Key(spec) => match tap_key(spec.modifier, spec.keycode) {
            Ok(()) => format!("ok key {}", spec.name),
            Err(error) => format!("err key {}: {error}", spec.name),
        },
        Command::Mouse { dx, dy } => {
            match tinyusb_hid::mouse_report(0, dx, dy) {
                Ok(()) => format!("ok mouse {dx} {dy}"),
                Err(error) => format!("err mouse: {error}"),
            }
        }
        Command::Click(button) => match click(button) {
            Ok(()) => format!("ok click {}", button_name(button)),
            Err(error) => format!("err click: {error}"),
        },
        Command::Media(spec) => match tap_media(spec.usage_code) {
            Ok(()) => format!("ok media {}", spec.name),
            Err(error) => format!("err media {}: {error}", spec.name),
        },
        Command::Help => protocol::HELP_TEXT.replace('\n', "\r\n"),
        Command::Invalid(reason) => format!("err {reason}"),
    }
}

fn tap_key(modifier: u8, keycode: u8) -> Result<(), esp_idf_sys::EspError> {
    tinyusb_hid::keyboard_press(modifier, keycode)?;
    thread::sleep(TAP_HOLD);
    tinyusb_hid::keyboard_release()
}

fn click(button: MouseButton) -> Result<(), esp_idf_sys::EspError> {
    tinyusb_hid::mouse_report(button.bit(), 0, 0)?;
    thread::sleep(TAP_HOLD);
    tinyusb_hid::mouse_report(0, 0, 0)
}

fn tap_media(usage_code: u16) -> Result<(), esp_idf_sys::EspError> {
    tinyusb_hid::consumer_press(usage_code)?;
    thread::sleep(TAP_HOLD);
    tinyusb_hid::consumer_release()
}

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
}
