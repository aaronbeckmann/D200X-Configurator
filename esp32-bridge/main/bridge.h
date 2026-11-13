// ESP32 bridge for the Ulanzi D200X: an agent for deckd on the deck's console UART.
// Protocol: esp32-bridge/docs/INTERFACE.md. Board: Seeed XIAO ESP32S3.
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "cJSON.h"
#include "esp_err.h"

#define BRIDGE_VERSION "0.3.0"
#define BRIDGE_AGENT "esp32-bridge " BRIDGE_VERSION

// XIAO ESP32S3 pins. UART1 on D0/D1: D6/D7 are UART0, where the ROM prints its boot log.
#define PIN_DECK_RX 1   // D0 <- deck console TX (pin next to TP3)
#define PIN_DECK_TX 2   // D1 -> deck console RX (far pin)
#define PIN_LED 21      // user LED, active low

// ---- state store (store.c) ----------------------------------------------------------------
void store_init(void);
// Set a state; it goes to the deck at the next flush if the deck wants it and it changed.
void store_set(const char *name, const char *value);
// Value of a state into out; false if unknown.
bool store_get(const char *name, char *out, size_t n);
// The deck's wanted list from hello (array of strings). Bumps the wanted version.
void store_set_wanted(const cJSON *states);
bool store_wanted(const char *name);
// Copy of the wanted names starting with prefix (e.g. "ha:"); caller frees with cJSON_Delete.
cJSON *store_wanted_with_prefix(const char *prefix);
// Changes whenever hello brings a new list; plugins resubscribe when it moves.
uint32_t store_wanted_version(void);
// Send every wanted state again (after hello).
void store_mark_all_dirty(void);
// A JSON object of changed wanted states, at most max_bytes long; NULL when nothing is due.
char *store_take_dirty_json(size_t max_bytes);
// Every known state whose name starts with prefix ("" = all), wanted or not, for the
// configurator's state explorer. Stops before max_bytes and says so in *truncated.
cJSON *store_dump_json(const char *prefix, size_t max_bytes, bool *truncated);

// ---- config (cfg.c): one JSON document in NVS ----------------------------------------------
void cfg_init(void);
// Section of the config ("wifi", "homeassistant", "mqtt", "net", "obs", "spotify", "connections").
// Valid until the next restart; never modify.
const cJSON *cfg_section(const char *name);
const char *cfg_str(const cJSON *obj, const char *key, const char *def);
// Validate and store a new config document. The caller restarts the chip afterwards.
bool cfg_save(const char *json, char *err, size_t errlen);
// Store one string in a section (rotated Spotify refresh token).
void cfg_update_string(const char *section, const char *key, const char *value);
// The config with secrets replaced by "***" (caller frees).
cJSON *cfg_redacted(void);

// ---- Wi-Fi (wifi.c) -----------------------------------------------------------------------
void wifi_start(void);
bool wifi_up(void);
// {"ssid","connected","rssi","ip"} (caller frees).
cJSON *wifi_status(void);

// ---- deck link (link.c) -------------------------------------------------------------------
void link_start(void);
bool link_session(void);
// Log to the USB console and, while the deck listens, into the deck's log (0x0213).
void link_log(const char *level, const char *fmt, ...) __attribute__((format(printf, 2, 3)));
// Status document (0x0223 and the console's "status"); caller frees.
cJSON *link_status_json(void);
void link_schedule_restart(int ms);

// ---- USB console (console.c): "config {...}", "status", "reboot" ---------------------------
void console_start(void);

// ---- plugins ------------------------------------------------------------------------------
// Built-in plugins are static instances; config-defined ones (plugin_decl.c) share one set of
// functions and tell themselves apart by `self` / `ctx`.
typedef struct plugin plugin_t;
struct plugin {
    const char *name;     // plugin name in deck actions and in the heartbeat
    const char *prefix;   // state prefix without ':' (NULL: no states)
    bool (*enabled)(const plugin_t *self);
    void (*start)(const plugin_t *self);
    bool (*ready)(const plugin_t *self);
    // Run one action; false with err filled on failure. delta: knob detents (summed when coalesced).
    bool (*run)(const plugin_t *self, const char *op, const cJSON *args, const char *event, int delta, char *err, size_t errlen);
    void *ctx;
};

extern const plugin_t plugin_homeassistant, plugin_mqtt, plugin_net, plugin_obs, plugin_spotify;
// Built-ins plus the config's "plugins" section.
const plugin_t *const *plugins_all(size_t *count);
const plugin_t *plugin_find(const char *name);

// ---- config-defined plugins (plugin_decl.c) ---------------------------------------------------
// Instances for the config's "plugins" section; *count set. Names taken by built-ins are skipped.
plugin_t *decl_plugins_create(size_t *count);
// MQTT topics declared by config-defined plugin states: called by the MQTT plugin.
void decl_mqtt_subscribe(void (*sub)(const char *topic));
void decl_mqtt_message(const char *topic, int topic_len, const char *payload);
// MQTT publish for config-defined ops (plugin_mqtt.c); false if MQTT is not connected.
bool mqtt_publish(const char *topic, const char *payload, int qos, bool retain);
bool mqtt_topic_matches(const char *filter, const char *topic, int topic_len);

// ---- over-the-air update (ota.c) ----------------------------------------------------------------
// Download and install an image from an http(s) URL in the background, then restart.
bool ota_start(const char *url, char *err, size_t errlen);
// Confirm the running image once it works (Wi-Fi up or 60 s), so the bootloader keeps it.
void ota_confirm_task_start(void);

// ---- shared helpers (util.c) --------------------------------------------------------------
// HTTP request; the body (NUL-terminated, at most 64 KB) is returned in *out (caller frees).
// headers: JSON object of extra headers, may be NULL. Returns the HTTP status or -1.
int http_request(const char *method, const char *url, const cJSON *headers, const char *content_type, const char *body, char **out);
// Follow a JSON pointer ("/a/0/b") and render the value as text ("" when missing).
void json_pointer_text(const cJSON *root, const char *pointer, char *out, size_t n);
// A JSON value as deck state text: strings raw, everything else compact JSON.
void json_value_text(const cJSON *v, char *out, size_t n);
void base64_encode(const uint8_t *in, size_t len, char *out, size_t outlen);
int64_t now_ms(void);
