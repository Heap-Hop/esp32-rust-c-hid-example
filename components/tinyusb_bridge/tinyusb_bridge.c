#include "tinyusb_bridge.h"

#include "class/hid/hid_device.h"
#include "esp_check.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "tinyusb.h"
#include "tinyusb_default_config.h"

#define HID_REPORT_ID_KEYBOARD 1
#define HID_REPORT_ID_MOUSE    2
#define HID_REPORT_ID_CONSUMER 3

#define HID_READY_TIMEOUT_MS 120
#define HID_READY_POLL_MS    2
#define HID_SEND_RETRY_COUNT 6

#define TUSB_DESC_TOTAL_LEN \
    (TUD_CONFIG_DESC_LEN + CFG_TUD_HID * TUD_HID_DESC_LEN)

static const char *TAG = "tinyusb_bridge";
static bool s_initialized = false;

/*
 * HID report descriptor — keyboard + mouse + consumer control (media keys),
 * each on its own report ID so the host can distinguish them.
 */
static const uint8_t s_hid_report_descriptor[] = {
    TUD_HID_REPORT_DESC_KEYBOARD(HID_REPORT_ID(HID_REPORT_ID_KEYBOARD)),
    TUD_HID_REPORT_DESC_MOUSE(HID_REPORT_ID(HID_REPORT_ID_MOUSE)),
    TUD_HID_REPORT_DESC_CONSUMER(HID_REPORT_ID(HID_REPORT_ID_CONSUMER)),
};

/*
 * USB string descriptors. Index 0 is the supported language list (0x0409 =
 * English US). The rest are picked up by TinyUSB when the host asks for
 * vendor / product / serial / interface names.
 */
static const char *s_hid_string_descriptor[] = {
    (char[]){0x09, 0x04},
    "esp32-rust-c-hid-example",
    "ESP32-S3 HID Demo",
    "esp32s3-hid-demo",
    "HID Interface",
};

static const uint8_t s_hid_configuration_descriptor[] = {
    TUD_CONFIG_DESCRIPTOR(1, 1, 0, TUSB_DESC_TOTAL_LEN,
                          TUSB_DESC_CONFIG_ATT_REMOTE_WAKEUP, 100),
    TUD_HID_DESCRIPTOR(0, 4, false, sizeof(s_hid_report_descriptor), 0x81, 16,
                       10),
};

/*
 * TinyUSB asks the application for the report descriptor at enumeration time.
 * This callback is required; without it the device never enumerates as HID.
 */
uint8_t const *tud_hid_descriptor_report_cb(uint8_t instance) {
    (void)instance;
    return s_hid_report_descriptor;
}

/*
 * tud_hid_get_report_cb and tud_hid_set_report_cb are other required TinyUSB
 * callbacks. They are implemented in Rust via #[no_mangle] (see
 * src/tinyusb_hid.rs), which keeps the FFI surface fully reviewable from
 * Rust. No C stubs needed for those.
 */

esp_err_t tinyusb_bridge_init(void) {
    if (s_initialized) {
        return ESP_OK;
    }

    tinyusb_config_t tusb_cfg = TINYUSB_DEFAULT_CONFIG();
    tusb_cfg.descriptor.device = NULL;
    tusb_cfg.descriptor.full_speed_config = s_hid_configuration_descriptor;
    tusb_cfg.descriptor.string = s_hid_string_descriptor;
    tusb_cfg.descriptor.string_count =
        sizeof(s_hid_string_descriptor) / sizeof(s_hid_string_descriptor[0]);
#if (TUD_OPT_HIGH_SPEED)
    tusb_cfg.descriptor.high_speed_config = s_hid_configuration_descriptor;
#endif

    ESP_RETURN_ON_ERROR(tinyusb_driver_install(&tusb_cfg), TAG,
                        "TinyUSB driver install failed");
    s_initialized = true;
    ESP_LOGI(TAG, "TinyUSB HID bridge initialized");
    return ESP_OK;
}

bool tinyusb_bridge_ready(void) {
    return s_initialized && tud_mounted() && tud_hid_ready();
}

static esp_err_t wait_hid_ready(TickType_t timeout_ticks) {
    TickType_t start = xTaskGetTickCount();
    while (!tinyusb_bridge_ready()) {
        if ((xTaskGetTickCount() - start) >= timeout_ticks) {
            return ESP_ERR_TIMEOUT;
        }
        vTaskDelay(pdMS_TO_TICKS(HID_READY_POLL_MS));
    }
    return ESP_OK;
}

static esp_err_t send_keyboard(uint8_t modifier, uint8_t const *keycodes) {
    for (int attempt = 0; attempt < HID_SEND_RETRY_COUNT; ++attempt) {
        ESP_RETURN_ON_ERROR(
            wait_hid_ready(pdMS_TO_TICKS(HID_READY_TIMEOUT_MS)), TAG,
            "HID keyboard not ready");
        if (tud_hid_keyboard_report(HID_REPORT_ID_KEYBOARD, modifier,
                                    keycodes)) {
            return ESP_OK;
        }
        vTaskDelay(pdMS_TO_TICKS(HID_READY_POLL_MS));
    }
    return ESP_FAIL;
}

static esp_err_t send_mouse(uint8_t buttons, int8_t dx, int8_t dy) {
    for (int attempt = 0; attempt < HID_SEND_RETRY_COUNT; ++attempt) {
        ESP_RETURN_ON_ERROR(
            wait_hid_ready(pdMS_TO_TICKS(HID_READY_TIMEOUT_MS)), TAG,
            "HID mouse not ready");
        if (tud_hid_mouse_report(HID_REPORT_ID_MOUSE, buttons, dx, dy, 0, 0)) {
            return ESP_OK;
        }
        vTaskDelay(pdMS_TO_TICKS(HID_READY_POLL_MS));
    }
    return ESP_FAIL;
}

static esp_err_t send_consumer(uint16_t usage_code) {
    for (int attempt = 0; attempt < HID_SEND_RETRY_COUNT; ++attempt) {
        ESP_RETURN_ON_ERROR(
            wait_hid_ready(pdMS_TO_TICKS(HID_READY_TIMEOUT_MS)), TAG,
            "HID consumer not ready");
        if (tud_hid_report(HID_REPORT_ID_CONSUMER, &usage_code,
                           sizeof(usage_code))) {
            return ESP_OK;
        }
        vTaskDelay(pdMS_TO_TICKS(HID_READY_POLL_MS));
    }
    return ESP_FAIL;
}

esp_err_t tinyusb_bridge_keyboard_press(uint8_t modifier, uint8_t keycode) {
    uint8_t keycodes[6] = {keycode, 0, 0, 0, 0, 0};
    return send_keyboard(modifier, keycodes);
}

esp_err_t tinyusb_bridge_keyboard_release(void) {
    uint8_t keycodes[6] = {0, 0, 0, 0, 0, 0};
    esp_err_t status = send_keyboard(0, keycodes);
    if (status == ESP_OK) {
        vTaskDelay(pdMS_TO_TICKS(12));
    }
    return status;
}

esp_err_t tinyusb_bridge_mouse_report(uint8_t buttons, int8_t dx, int8_t dy) {
    return send_mouse(buttons, dx, dy);
}

esp_err_t tinyusb_bridge_consumer_press(uint16_t usage_code) {
    return send_consumer(usage_code);
}

esp_err_t tinyusb_bridge_consumer_release(void) {
    esp_err_t status = send_consumer(0);
    if (status == ESP_OK) {
        vTaskDelay(pdMS_TO_TICKS(12));
    }
    return status;
}
