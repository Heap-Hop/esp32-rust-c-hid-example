//! ESP32-S3 TinyUSB HID over UDP example.
//!
//! At boot:
//!   1. Install the TinyUSB HID bridge so the device enumerates as a keyboard
//!      + mouse on the USB port.
//!   2. Connect to Wi-Fi using credentials baked in at build time.
//!   3. Listen on a UDP port; each datagram is one newline-optional text
//!      command (see `protocol.rs`) and turned into a HID report. The sender
//!      receives a single-line `ok ...` / `err ...` reply.
//!
//! Test from any host on the same network:
//!   nc -u <board-ip> 9000
//!   key a
//!   mouse 20 -5
//!   media volup
//!
//! Neither `main.rs` nor any other file outside `tinyusb_hid.rs` contains an
//! `unsafe` block — the FFI is fully encapsulated there.

use std::net::UdpSocket;
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
const BUILT_UDP_PORT: &str = env!("UDP_PORT_BUILT");

/// Delay between key press and key release for a "tap".
const TAP_HOLD: Duration = Duration::from_millis(12);
/// Largest datagram we accept. Anything beyond is silently truncated by the
/// kernel; we just use a generous local buffer.
const MAX_DATAGRAM_BYTES: usize = 1500;

fn main() -> anyhow::Result<()> {
    // Required ESP-IDF setup — always before anything else.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let udp_port: u16 = BUILT_UDP_PORT
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid UDP_PORT '{BUILT_UDP_PORT}': {e}"))?;
    log::info!(
        "esp32-rust-c-hid-example starting (SSID='{}', UDP port={})",
        BUILT_WIFI_SSID,
        udp_port,
    );

    tinyusb_hid::init()?;
    log::info!("TinyUSB HID bridge initialised");

    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    // Keep the Wi-Fi guard alive for the whole program.
    let _wifi = connect_wifi(peripherals, sysloop, nvs)?;

    run_udp_server(udp_port)
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

fn run_udp_server(port: u16) -> anyhow::Result<()> {
    let socket = UdpSocket::bind(("0.0.0.0", port))?;
    log::info!("UDP HID command server listening on 0.0.0.0:{port}");

    let mut buffer = vec![0_u8; MAX_DATAGRAM_BYTES];
    loop {
        let (len, peer) = match socket.recv_from(&mut buffer) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("udp recv error: {error}");
                continue;
            }
        };

        let payload = &buffer[..len];
        let text = match std::str::from_utf8(payload) {
            Ok(s) => s,
            Err(_) => {
                let _ = socket.send_to(b"err invalid utf-8\n", peer);
                continue;
            }
        };

        // Accept multi-line datagrams just in case a client batches commands.
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let reply = handle_line(trimmed);
            let mut reply_bytes = reply.into_bytes();
            reply_bytes.push(b'\n');
            if let Err(error) = socket.send_to(&reply_bytes, peer) {
                log::warn!("udp send to {peer} failed: {error}");
                break;
            }
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
    std::thread::sleep(TAP_HOLD);
    tinyusb_hid::keyboard_release()
}

fn click(button: MouseButton) -> Result<(), esp_idf_sys::EspError> {
    tinyusb_hid::mouse_report(button.bit(), 0, 0)?;
    std::thread::sleep(TAP_HOLD);
    tinyusb_hid::mouse_report(0, 0, 0)
}

fn tap_media(usage_code: u16) -> Result<(), esp_idf_sys::EspError> {
    tinyusb_hid::consumer_press(usage_code)?;
    std::thread::sleep(TAP_HOLD);
    tinyusb_hid::consumer_release()
}

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
}
