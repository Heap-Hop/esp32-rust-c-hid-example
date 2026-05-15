//! ESP32-S3 TinyUSB HID over UDP.
//!
//! At boot:
//!   1. Install the TinyUSB HID bridge so the device enumerates as a keyboard
//!      + mouse on the USB port.
//!   2. Connect to Wi-Fi using credentials baked in at build time.
//!   3. Listen on a UDP port; each datagram is one fixed binary command (see
//!      `protocol.rs`) and is turned into a HID report.
//!
//! Commands are fire-and-forget. The firmware only replies to `PING`, with a
//! matching `PONG`, so a client can probe reachability without flooding the
//! network for every keystroke or mouse delta.
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

use protocol::{Command, MAX_REPLY_LEN, ParseError};

// Baked-in build-time configuration (see build.rs).
const BUILT_WIFI_SSID: &str = env!("WIFI_SSID_BUILT");
const BUILT_WIFI_PASSWORD: &str = env!("WIFI_PASSWORD_BUILT");
const BUILT_UDP_PORT: &str = env!("UDP_PORT_BUILT");

/// Delay between press and release for a "tap" (key or mouse click).
const TAP_HOLD: Duration = Duration::from_millis(12);
/// Largest datagram we accept. Anything beyond is silently truncated by the
/// kernel; we just use a generous local buffer.
const MAX_DATAGRAM_BYTES: usize = 256;

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

    let mut buffer = [0_u8; MAX_DATAGRAM_BYTES];
    loop {
        let (len, peer) = match socket.recv_from(&mut buffer) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("udp recv error: {error}");
                continue;
            }
        };

        match protocol::parse(&buffer[..len]) {
            Ok(Command::Ping { seq }) => {
                let mut reply = [0u8; MAX_REPLY_LEN];
                let n = protocol::write_pong(seq, &mut reply);
                if let Err(error) = socket.send_to(&reply[..n], peer) {
                    log::warn!("pong send to {peer} failed: {error}");
                }
            }
            Ok(command) => {
                if let Err(error) = execute(command) {
                    log::warn!("hid error executing {command:?}: {error}");
                }
            }
            Err(error) => {
                // Random Internet noise hits port 9000 occasionally; only log
                // packets that *look* like they were meant for us.
                if matches!(error, ParseError::BadMagic(_)) {
                    continue;
                }
                log::warn!("rejected datagram from {peer}: {error:?}");
            }
        }
    }
}

fn execute(command: Command) -> Result<(), esp_idf_sys::EspError> {
    match command {
        Command::KeyTap { modifier, keycode } => tap_key(modifier, keycode),
        Command::KeyDown { modifier, keycode } => {
            tinyusb_hid::keyboard_press(modifier, keycode)
        }
        Command::KeyUp => tinyusb_hid::keyboard_release(),
        Command::MouseMove { dx, dy, wheel: _ } => {
            // The current HID descriptor in components/tinyusb_bridge does not
            // include a wheel axis, so wheel is intentionally ignored. The
            // wire format carries it for forward compatibility.
            tinyusb_hid::mouse_report(0, dx, dy)
        }
        Command::MouseClick { buttons } => mouse_click(buttons),
        Command::MouseButtons { buttons } => tinyusb_hid::mouse_report(buttons, 0, 0),
        Command::MediaTap { usage } => tap_media(usage),
        // PING is handled above the dispatcher so it can read the peer addr.
        Command::Ping { .. } => unreachable!(),
    }
}

fn tap_key(modifier: u8, keycode: u8) -> Result<(), esp_idf_sys::EspError> {
    tinyusb_hid::keyboard_press(modifier, keycode)?;
    std::thread::sleep(TAP_HOLD);
    tinyusb_hid::keyboard_release()
}

fn mouse_click(buttons: u8) -> Result<(), esp_idf_sys::EspError> {
    tinyusb_hid::mouse_report(buttons, 0, 0)?;
    std::thread::sleep(TAP_HOLD);
    tinyusb_hid::mouse_report(0, 0, 0)
}

fn tap_media(usage: u16) -> Result<(), esp_idf_sys::EspError> {
    tinyusb_hid::consumer_press(usage)?;
    std::thread::sleep(TAP_HOLD);
    tinyusb_hid::consumer_release()
}
