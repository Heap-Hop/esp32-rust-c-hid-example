idf_component_register(
    SRCS "c_example.c"
    INCLUDE_DIRS "include"
    # Add REQUIRES / PRIV_REQUIRES here for any ESP-IDF components this one depends on.
    # Examples:
    #   REQUIRES     esp_wifi esp_http_client
    #   PRIV_REQUIRES esp_driver_gpio
)
