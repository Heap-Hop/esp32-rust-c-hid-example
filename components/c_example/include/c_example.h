#pragma once

/*
 * c_example — minimal C component shipped with the esp32-rust-c-template.
 *
 * Rules for bindgen-safe headers (so Rust can call these without any shims):
 *   - Use only plain C types: stdint.h integers, bool, esp_err_t, char*.
 *   - Avoid C++ features, complex macros, and non-standard extensions.
 *   - Guard with extern "C" so C++ callers also work.
 *   - Keep each public function in this header; put private helpers in the .c file.
 */

#include "esp_err.h"
#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Initialize the component. Call once at startup before any other function. */
esp_err_t c_example_init(void);

/* Returns a + b. Replace with real logic that belongs in C (driver calls, etc.). */
int32_t c_example_add(int32_t a, int32_t b);

#ifdef __cplusplus
}
#endif
