use std::thread;
use std::time::Duration;

mod c_example;

fn main() {
    // ── Required ESP-IDF setup ────────────────────────────────────────────────
    // Always keep these two lines before anything else.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("firmware starting");

    // ── Call into C via safe wrappers ─────────────────────────────────────────
    // Each C component has a corresponding src/<component>.rs that exposes a
    // safe Rust API. No unsafe blocks needed here.
    c_example::init().expect("c_example init failed");

    let result = c_example::add(3, 4);
    log::info!("c_example::add(3, 4) = {result}");

    // ── Main loop ─────────────────────────────────────────────────────────────
    // Replace with your application logic: TCP listener, sensor polling, etc.
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
