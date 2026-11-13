idf_component_register(
    SRCS "main.c" "link.c" "store.c" "cfg.c" "wifi.c" "util.c" "console.c"
         "plugin_ha.c" "plugin_mqtt.c" "plugin_net.c" "plugin_obs.c" "plugin_spotify.c"
         "plugin_decl.c" "ota.c"
    INCLUDE_DIRS "."
    PRIV_REQUIRES esp_driver_uart esp_driver_gpio esp_driver_usb_serial_jtag esp_wifi esp_netif
                  esp_event nvs_flash esp_http_client esp_https_ota app_update esp_timer mbedtls lwip esp_psram vfs
)
