#pragma once

/*
 * tinyusb_bridge — thin C wrapper around TinyUSB HID device for Rust FFI.
 *
 * The header exposes only plain C types (esp_err_t, bool, intNN_t) so bindgen
 * (when used) and hand-written FFI in Rust can both consume it cleanly.
 *
 * The TinyUSB report descriptor is built by C macros that would be awkward to
 * expand in Rust, so that portion stays in C. All policy (which keys to press,
 * retries, error handling) lives on the Rust side.
 */

#include "esp_err.h"
#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Initialize the TinyUSB driver and install the HID report descriptor.
 * Call once at startup. Safe to call more than once — subsequent calls are a
 * no-op. */
esp_err_t tinyusb_bridge_init(void);

/* Returns true when the USB device has been mounted by the host and the HID
 * interface is ready to accept reports. */
bool tinyusb_bridge_ready(void);

/* Send a keyboard report with a single keycode and modifier bitmap. Follow
 * this with tinyusb_bridge_keyboard_release() to release the key. */
esp_err_t tinyusb_bridge_keyboard_press(uint8_t modifier, uint8_t keycode);

/* Send a keyboard report with all keys released. */
esp_err_t tinyusb_bridge_keyboard_release(void);

/* Mouse relative movement with per-axis deltas in the range -127..127, and a
 * button bitmap (bit 0 = left, bit 1 = right, bit 2 = middle). */
esp_err_t tinyusb_bridge_mouse_report(uint8_t buttons, int8_t dx, int8_t dy);

/* Press a consumer (media) control usage code. Release with
 * tinyusb_bridge_consumer_release(). */
esp_err_t tinyusb_bridge_consumer_press(uint16_t usage_code);

/* Release any pressed consumer control. */
esp_err_t tinyusb_bridge_consumer_release(void);

#ifdef __cplusplus
}
#endif
