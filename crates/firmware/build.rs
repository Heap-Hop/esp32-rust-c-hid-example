fn main() {
    // Emit the linker flags and environment variables that esp-idf-sys needs.
    embuild::espidf::sysenv::output();

    // Build-time Wi-Fi credentials and UDP port. Re-run the build script if any
    // of these change so the firmware is rebuilt with the new values.
    println!("cargo:rerun-if-env-changed=WIFI_SSID");
    println!("cargo:rerun-if-env-changed=WIFI_PASSWORD");
    println!("cargo:rerun-if-env-changed=UDP_PORT");

    let wifi_ssid = std::env::var("WIFI_SSID")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| panic!("WIFI_SSID must be set before building firmware"));
    // Empty password means open network — not an error.
    let wifi_password = std::env::var("WIFI_PASSWORD").unwrap_or_default();
    let udp_port = std::env::var("UDP_PORT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "9000".to_owned());

    println!("cargo:rustc-env=WIFI_SSID_BUILT={wifi_ssid}");
    println!("cargo:rustc-env=WIFI_PASSWORD_BUILT={wifi_password}");
    println!("cargo:rustc-env=UDP_PORT_BUILT={udp_port}");
}
