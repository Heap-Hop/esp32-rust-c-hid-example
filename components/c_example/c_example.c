#include "c_example.h"
#include "esp_log.h"

static const char *TAG = "c_example";

esp_err_t c_example_init(void) {
    ESP_LOGI(TAG, "initialized");
    return ESP_OK;
}

int32_t c_example_add(int32_t a, int32_t b) {
    return a + b;
}
