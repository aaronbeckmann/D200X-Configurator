// Link to deckd over the deck's console UART (esp32-bridge/docs/INTERFACE.md sections 2 and 3).
//
// Safety rule: the deck's console is a root shell whenever deckd is not running, so our TX pin
// stays high impedance until a valid deckd beacon arrives, and goes back to high impedance when
// beacons stop for 3 s.
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "driver/gpio.h"
#include "driver/uart.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/semphr.h"
#include "freertos/task.h"

#define UARTN UART_NUM_1
#define MAX_PAYLOAD 16384
#define HEADER 8
#define RX_CAP (MAX_PAYLOAD + HEADER + 2 + 512)
#define STATES_MAX 2048  // bytes per 0x0211 frame
#define STATES_DUMP_MAX 12000  // "states" command reply; frames are at most 16384 bytes
#define MAX_COALESCE 16

static const char *TAG = "bridge";

static uint8_t *rx;
static size_t rx_len;
static int64_t rx_last;
static uint32_t rx_errors;

static SemaphoreHandle_t tx_lock;
static QueueHandle_t actions;
static volatile bool session, hello_seen;
static char last_boot[24];
static char deck_version[24];
static int64_t last_beacon, last_hb, last_flush, started_ms;

// ---- frames ---------------------------------------------------------------------------------

static uint16_t crc16_update(uint16_t crc, const uint8_t *d, size_t n) {
    for (size_t i = 0; i < n; i++) {
        crc ^= (uint16_t)d[i] << 8;
        for (int b = 0; b < 8; b++) {
            crc = crc & 0x8000 ? (crc << 1) ^ 0x1021 : crc << 1;
        }
    }
    return crc;
}

static uint32_t le32(const uint8_t *b) {
    return b[4] | b[5] << 8 | b[6] << 16 | (uint32_t)b[7] << 24;
}

static bool complete_at(const uint8_t *b, size_t n) {
    if (n < HEADER + 2 || b[0] != 0x7c || b[1] != 0x7c) return false;
    uint32_t len = le32(b);
    if (len > MAX_PAYLOAD || n < HEADER + len + 2) return false;
    return crc16_update(0xffff, b + 2, HEADER - 2 + len) == (b[HEADER + len] << 8 | b[HEADER + len + 1]);
}

static void drop_front(size_t k) {
    memmove(rx, rx + k, rx_len - k);
    rx_len -= k;
}

static void handle_frame(uint16_t cmd, const uint8_t *p, size_t n);

static void parse(const uint8_t *data, size_t n) {
    int64_t t = now_ms();
    if (rx_len && t - rx_last > 1000) {
        drop_front(1);  // a frame that never completed
        rx_errors++;
    }
    if (n) {
        rx_last = t;
        if (rx_len + n > RX_CAP) {
            drop_front(rx_len + n - RX_CAP < rx_len ? rx_len + n - RX_CAP : rx_len);
        }
        memcpy(rx + rx_len, data, n);
        rx_len += n;
    }
    for (;;) {
        size_t i = 0;
        while (i + 1 < rx_len && !(rx[i] == 0x7c && rx[i + 1] == 0x7c)) i++;
        if (i + 1 >= rx_len) {
            bool keep = rx_len && rx[rx_len - 1] == 0x7c;
            rx_len = 0;
            if (keep) rx[rx_len++] = 0x7c;
            return;
        }
        drop_front(i);
        if (rx_len < HEADER) return;
        uint32_t len = le32(rx);
        if (len > MAX_PAYLOAD) {
            drop_front(1);
            rx_errors++;
            continue;
        }
        size_t total = HEADER + len + 2;
        if (rx_len < total) {
            // a false start may hide a real frame behind it
            size_t j = 1;
            while (j < rx_len && !complete_at(rx + j, rx_len - j)) j++;
            if (j < rx_len) {
                drop_front(j);
                rx_errors++;
                continue;
            }
            return;
        }
        if (!complete_at(rx, rx_len)) {
            drop_front(1);
            rx_errors++;
            continue;
        }
        uint16_t cmd = rx[2] << 8 | rx[3];
        handle_frame(cmd, rx + HEADER, len);
        drop_front(total);
    }
}

static void send_frame(uint16_t cmd, const char *payload, size_t n) {
    if (!session || !tx_lock) return;
    uint8_t h[HEADER] = {0x7c, 0x7c, cmd >> 8, cmd & 0xff, n & 0xff, n >> 8 & 0xff, n >> 16 & 0xff, n >> 24 & 0xff};
    uint16_t crc = crc16_update(crc16_update(0xffff, h + 2, HEADER - 2), (const uint8_t *)payload, n);
    uint8_t c[2] = {crc >> 8, crc & 0xff};
    xSemaphoreTake(tx_lock, portMAX_DELAY);
    uart_write_bytes(UARTN, h, HEADER);
    uart_write_bytes(UARTN, payload, n);
    uart_write_bytes(UARTN, c, 2);
    xSemaphoreGive(tx_lock);
}

static void send_json(uint16_t cmd, cJSON *o) {
    char *s = cJSON_PrintUnformatted(o);
    if (s) {
        send_frame(cmd, s, strlen(s));
        free(s);
    }
}

// ---- TX pin gating ---------------------------------------------------------------------------

static void tx_attach(bool on) {
    if (on) {
        uart_set_pin(UARTN, PIN_DECK_TX, PIN_DECK_RX, UART_PIN_NO_CHANGE, UART_PIN_NO_CHANGE);
    } else {
        uart_wait_tx_done(UARTN, pdMS_TO_TICKS(200));
        gpio_reset_pin(PIN_DECK_TX);
        gpio_set_direction(PIN_DECK_TX, GPIO_MODE_INPUT);
        gpio_set_pull_mode(PIN_DECK_TX, GPIO_FLOATING);
    }
}

// ---- messages --------------------------------------------------------------------------------

void link_log(const char *level, const char *fmt, ...) {
    char msg[200];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(msg, sizeof msg, fmt, ap);
    va_end(ap);
    if (!strcmp(level, "error")) ESP_LOGE(TAG, "%s", msg);
    else if (!strcmp(level, "warn")) ESP_LOGW(TAG, "%s", msg);
    else ESP_LOGI(TAG, "%s", msg);
    if (session && strcmp(level, "debug")) {
        cJSON *o = cJSON_CreateObject();
        cJSON_AddStringToObject(o, "level", level);
        cJSON_AddStringToObject(o, "msg", msg);
        send_json(0x0213, o);
        cJSON_Delete(o);
    }
}

bool link_session(void) {
    return session;
}

static void send_heartbeat(void) {
    cJSON *o = cJSON_CreateObject();
    cJSON_AddStringToObject(o, "agent", BRIDGE_AGENT);
    cJSON *plugins = cJSON_AddArrayToObject(o, "plugins");
    cJSON *ready = cJSON_AddObjectToObject(o, "ready");
    cJSON *prefixes = cJSON_AddObjectToObject(o, "prefixes");
    size_t n;
    const plugin_t *const *all = plugins_all(&n);
    for (size_t i = 0; i < n; i++) {
        if (!all[i]->enabled(all[i])) continue;
        cJSON_AddItemToArray(plugins, cJSON_CreateString(all[i]->name));
        cJSON_AddBoolToObject(ready, all[i]->name, all[i]->ready(all[i]));
        if (all[i]->prefix) cJSON_AddStringToObject(prefixes, all[i]->prefix, all[i]->name);
    }
    send_json(0x0210, o);
    cJSON_Delete(o);
}

static void send_result(double id, bool ok, const char *err) {
    cJSON *o = cJSON_CreateObject();
    cJSON_AddNumberToObject(o, "id", id);
    cJSON_AddBoolToObject(o, "ok", ok);
    if (!ok) cJSON_AddStringToObject(o, "error", err);
    send_json(0x0212, o);
    cJSON_Delete(o);
}

cJSON *link_status_json(void) {
    cJSON *o = cJSON_CreateObject();
    cJSON_AddStringToObject(o, "version", BRIDGE_VERSION);
    cJSON_AddNumberToObject(o, "uptime_s", (double)(now_ms() / 1000));
    cJSON_AddItemToObject(o, "wifi", wifi_status());
    cJSON *deck = cJSON_AddObjectToObject(o, "deck");
    cJSON_AddBoolToObject(deck, "session", session);
    cJSON_AddStringToObject(deck, "deckd", deck_version);
    cJSON_AddNumberToObject(deck, "rx_errors", rx_errors);
    cJSON *pl = cJSON_AddObjectToObject(o, "plugins");
    size_t n;
    const plugin_t *const *all = plugins_all(&n);
    for (size_t i = 0; i < n; i++) {
        cJSON *p = cJSON_AddObjectToObject(pl, all[i]->name);
        cJSON_AddBoolToObject(p, "enabled", all[i]->enabled(all[i]));
        cJSON_AddBoolToObject(p, "ready", all[i]->enabled(all[i]) && all[i]->ready(all[i]));
    }
    cJSON_AddNumberToObject(o, "heap_free", heap_caps_get_free_size(MALLOC_CAP_INTERNAL));
    cJSON_AddNumberToObject(o, "psram_free", heap_caps_get_free_size(MALLOC_CAP_SPIRAM));
    cJSON_AddItemToObject(o, "config", cfg_redacted());
    return o;
}

static void restart_cb(void *arg) {
    esp_restart();
}

void link_schedule_restart(int ms) {
    static esp_timer_handle_t t;
    if (!t) {
        esp_timer_create_args_t a = {.callback = restart_cb, .name = "restart"};
        esp_timer_create(&a, &t);
    }
    esp_timer_start_once(t, (uint64_t)ms * 1000);
}

static void handle_frame(uint16_t cmd, const uint8_t *p, size_t n) {
    char *s = malloc(n + 1);
    if (!s) return;
    memcpy(s, p, n);
    s[n] = 0;
    cJSON *j = cJSON_Parse(s);
    free(s);
    switch (cmd) {
    case 0x0312: {  // beacon
        const char *boot = cJSON_GetStringValue(cJSON_GetObjectItem(j, "boot"));
        last_beacon = now_ms();
        if (!session) {
            session = true;
            tx_attach(true);
            snprintf(deck_version, sizeof deck_version, "%s", cJSON_GetStringValue(cJSON_GetObjectItem(j, "version")) ?: "?");
            last_hb = 0;  // heartbeat right away
            link_log("info", "deck: deckd %s, link up", deck_version);
        }
        if (boot && strcmp(boot, last_boot)) {
            if (last_boot[0]) link_log("info", "deck: deckd restarted");
            snprintf(last_boot, sizeof last_boot, "%s", boot);
            hello_seen = false;
        }
        break;
    }
    case 0x0311: {  // hello
        const cJSON *states = cJSON_GetObjectItem(j, "states");
        store_set_wanted(states);
        store_mark_all_dirty();
        hello_seen = true;
        link_log("info", "deck: hello, %d states wanted", cJSON_GetArraySize(states));
        break;
    }
    case 0x0310:  // host action
        if (j && xQueueSend(actions, &j, 0) == pdTRUE) {
            j = NULL;  // the worker owns it now
        } else {
            send_result(cJSON_GetNumberValue(cJSON_GetObjectItem(j, "id")), false, "bridge busy");
        }
        break;
    case 0x0320: {  // config from the configurator, relayed by deckd
        cJSON *r = cJSON_CreateObject();
        cJSON_AddItemToObject(r, "req", cJSON_Duplicate(cJSON_GetObjectItem(j, "req"), true));
        char *c = cJSON_PrintUnformatted(cJSON_GetObjectItem(j, "config"));
        char err[96] = "";
        bool ok = c && cfg_save(c, err, sizeof err);
        free(c);
        cJSON_AddBoolToObject(r, "ok", ok);
        if (!ok) cJSON_AddStringToObject(r, "error", err[0] ? err : "missing config");
        send_json(0x0221, r);
        cJSON_Delete(r);
        if (ok) {
            link_log("info", "config stored, restarting");
            link_schedule_restart(800);
        }
        break;
    }
    case 0x0322: {  // status
        cJSON *r = link_status_json();
        cJSON_AddItemToObject(r, "req", cJSON_Duplicate(cJSON_GetObjectItem(j, "req"), true));
        send_json(0x0223, r);
        cJSON_Delete(r);
        break;
    }
    case 0x0324: {  // command: {"req","cmd":"ota","url"} | {"cmd":"reboot"} | {"cmd":"states","prefix"}
        const char *c = cJSON_GetStringValue(cJSON_GetObjectItem(j, "cmd")) ?: "";
        if (!strcmp(c, "states")) {
            // everything the plugins know, for the configurator's state explorer
            cJSON *r = cJSON_CreateObject();
            cJSON_AddItemToObject(r, "req", cJSON_Duplicate(cJSON_GetObjectItem(j, "req"), true));
            cJSON_AddBoolToObject(r, "ok", true);
            cJSON *plugins = cJSON_AddArrayToObject(r, "plugins");
            size_t n;
            const plugin_t *const *all = plugins_all(&n);
            for (size_t i = 0; i < n; i++) {
                if (all[i]->enabled(all[i])) cJSON_AddItemToArray(plugins, cJSON_CreateString(all[i]->name));
            }
            bool truncated;
            cJSON_AddItemToObject(r, "states", store_dump_json(cJSON_GetStringValue(cJSON_GetObjectItem(j, "prefix")) ?: "", STATES_DUMP_MAX, &truncated));
            cJSON_AddBoolToObject(r, "truncated", truncated);
            send_json(0x0225, r);
            cJSON_Delete(r);
            break;
        }
        char err[96] = "";
        bool ok;
        if (!strcmp(c, "ota")) {
            ok = ota_start(cJSON_GetStringValue(cJSON_GetObjectItem(j, "url")), err, sizeof err);
        } else if (!strcmp(c, "reboot")) {
            ok = true;
            link_schedule_restart(500);
        } else {
            ok = false;
            snprintf(err, sizeof err, "unknown command %s", c);
        }
        cJSON *r = cJSON_CreateObject();
        cJSON_AddItemToObject(r, "req", cJSON_Duplicate(cJSON_GetObjectItem(j, "req"), true));
        cJSON_AddBoolToObject(r, "ok", ok);
        if (!ok) cJSON_AddStringToObject(r, "error", err);
        send_json(0x0225, r);
        cJSON_Delete(r);
        break;
    }
    default:
        break;  // 0x0314 baud reply and anything newer: nothing to do
    }
    cJSON_Delete(j);
}

// ---- action worker -----------------------------------------------------------------------------

static bool same_turn(const cJSON *a, const cJSON *b) {
    const char *ea = cJSON_GetStringValue(cJSON_GetObjectItem(a, "event"));
    const char *eb = cJSON_GetStringValue(cJSON_GetObjectItem(b, "event"));
    return ea && eb && !strcmp(ea, "turn") && !strcmp(eb, "turn") &&
           cJSON_Compare(cJSON_GetObjectItem(a, "plugin"), cJSON_GetObjectItem(b, "plugin"), true) &&
           cJSON_Compare(cJSON_GetObjectItem(a, "op"), cJSON_GetObjectItem(b, "op"), true) &&
           cJSON_Compare(cJSON_GetObjectItem(a, "args"), cJSON_GetObjectItem(b, "args"), true);
}

static void worker_task(void *arg) {
    for (;;) {
        cJSON *act;
        xQueueReceive(actions, &act, portMAX_DELAY);
        double ids[MAX_COALESCE];
        int nid = 0;
        ids[nid++] = cJSON_GetNumberValue(cJSON_GetObjectItem(act, "id"));
        int delta = (int)cJSON_GetNumberValue(cJSON_GetObjectItem(act, "delta"));
        // knob bursts: one call for all detents that queued up meanwhile
        cJSON *next;
        while (nid < MAX_COALESCE && xQueuePeek(actions, &next, 0) == pdTRUE && same_turn(act, next)) {
            xQueueReceive(actions, &next, 0);
            delta += (int)cJSON_GetNumberValue(cJSON_GetObjectItem(next, "delta"));
            ids[nid++] = cJSON_GetNumberValue(cJSON_GetObjectItem(next, "id"));
            cJSON_Delete(next);
        }
        const char *plugin = cJSON_GetStringValue(cJSON_GetObjectItem(act, "plugin")) ?: "";
        const char *op = cJSON_GetStringValue(cJSON_GetObjectItem(act, "op")) ?: "";
        const char *event = cJSON_GetStringValue(cJSON_GetObjectItem(act, "event")) ?: "press";
        const cJSON *args = cJSON_GetObjectItem(act, "args");
        const plugin_t *p = plugin_find(plugin);
        char err[160] = "";
        bool ok = false;
        if (!p || !p->enabled(p)) {
            snprintf(err, sizeof err, "plugin %s not configured on the bridge", plugin);
        } else if (!p->ready(p)) {
            snprintf(err, sizeof err, "%s not connected", plugin);
        } else {
            ok = p->run(p, op, args, event, delta, err, sizeof err);
        }
        for (int i = 0; i < nid; i++) {
            send_result(ids[i], ok, err);
        }
        if (!ok) link_log("warn", "%s.%s: %s", plugin, op, err);
        cJSON_Delete(act);
    }
}

// ---- main loop ---------------------------------------------------------------------------------

static void led(bool on) {
    gpio_set_level(PIN_LED, on ? 0 : 1);
}

static void link_task(void *arg) {
    uint8_t buf[512];
    for (;;) {
        int n = uart_read_bytes(UARTN, buf, sizeof buf, pdMS_TO_TICKS(20));
        parse(buf, n > 0 ? n : 0);
        int64_t t = now_ms();
        if (session && t - last_beacon > 3000) {
            session = false;
            hello_seen = false;
            tx_attach(false);
            ESP_LOGW(TAG, "deck: beacons stopped, TX released");
        }
        if (session && t - last_hb >= 2000) {
            last_hb = t;
            send_heartbeat();
        }
        if (session && hello_seen && t - last_flush >= 100) {
            last_flush = t;
            char *s = store_take_dirty_json(STATES_MAX);
            if (s) {
                send_frame(0x0211, s, strlen(s));
                free(s);
            }
        }
        // LED: off = no deck, blinking = deck but no Wi-Fi, on = all up
        led(session && (wifi_up() || (t / 500) % 2));
    }
}

void link_start(void) {
    started_ms = now_ms();
    rx = heap_caps_malloc(RX_CAP, MALLOC_CAP_8BIT);
    tx_lock = xSemaphoreCreateMutex();
    actions = xQueueCreate(24, sizeof(cJSON *));
    gpio_reset_pin(PIN_LED);
    gpio_set_direction(PIN_LED, GPIO_MODE_OUTPUT);
    led(false);
    uart_config_t c = {
        .baud_rate = 115200,
        .data_bits = UART_DATA_8_BITS,
        .parity = UART_PARITY_DISABLE,
        .stop_bits = UART_STOP_BITS_1,
        .flow_ctrl = UART_HW_FLOWCTRL_DISABLE,
        .source_clk = UART_SCLK_DEFAULT,
    };
    uart_driver_install(UARTN, 8192, 4096, 0, NULL, 0);
    uart_param_config(UARTN, &c);
    // RX only until deckd's beacon: our TX stays off the deck's console
    uart_set_pin(UARTN, UART_PIN_NO_CHANGE, PIN_DECK_RX, UART_PIN_NO_CHANGE, UART_PIN_NO_CHANGE);
    tx_attach(false);
    xTaskCreatePinnedToCore(link_task, "link", 6144, NULL, 8, NULL, 1);
    xTaskCreatePinnedToCore(worker_task, "actions", 8192, NULL, 5, NULL, 1);
    ESP_LOGI(TAG, "listening on UART1 RX=GPIO%d (D0), TX=GPIO%d (D1) held off", PIN_DECK_RX, PIN_DECK_TX);
}
