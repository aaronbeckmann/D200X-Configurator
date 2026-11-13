// ESP32 bridge for the Ulanzi D200X: a second agent for deckd on the deck's console UART.
// Runs network plugins (Home Assistant, MQTT, HTTP/WoL/UDP, OBS, Spotify, and plugins defined in
// the config) so they work without the PC agent. Wiring: esp32-bridge/docs/WIRING.md.
// Protocol: esp32-bridge/docs/INTERFACE.md.
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "esp_log.h"

static const plugin_t *BUILTIN[] = {&plugin_homeassistant, &plugin_mqtt, &plugin_net, &plugin_obs, &plugin_spotify};
#define BUILTIN_COUNT (sizeof BUILTIN / sizeof BUILTIN[0])

static const plugin_t **all;
static size_t all_count;

const plugin_t *const *plugins_all(size_t *count) {
    *count = all_count;
    return all;
}

const plugin_t *plugin_find(const char *name) {
    for (size_t i = 0; i < all_count; i++) {
        if (!strcmp(all[i]->name, name)) {
            return all[i];
        }
    }
    return NULL;
}

void app_main(void) {
    store_init();
    cfg_init();
    size_t decl_count = 0;
    plugin_t *decl = decl_plugins_create(&decl_count);
    all = calloc(BUILTIN_COUNT + decl_count, sizeof *all);
    for (size_t i = 0; i < BUILTIN_COUNT; i++) all[all_count++] = BUILTIN[i];
    for (size_t i = 0; i < decl_count; i++) all[all_count++] = &decl[i];

    link_start();
    console_start();
    wifi_start();
    ota_confirm_task_start();
    for (size_t i = 0; i < all_count; i++) {
        if (all[i]->enabled(all[i])) {
            ESP_LOGI("bridge", "plugin %s enabled", all[i]->name);
            all[i]->start(all[i]);
        }
    }
}
