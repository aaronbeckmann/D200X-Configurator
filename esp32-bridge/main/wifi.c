// Wi-Fi station from the "wifi" config section; reconnects with backoff.
#include <string.h>
#include "bridge.h"
#include "esp_event.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const char *TAG = "wifi";
static volatile bool up;
static esp_netif_t *netif;
static int retries;

static void retry_task(void *arg) {
    int delay = (int)(intptr_t)arg;
    vTaskDelay(pdMS_TO_TICKS(delay));
    esp_wifi_connect();
    vTaskDelete(NULL);
}

static void on_event(void *arg, esp_event_base_t base, int32_t id, void *data) {
    if (base == WIFI_EVENT && id == WIFI_EVENT_STA_START) {
        esp_wifi_connect();
    } else if (base == WIFI_EVENT && id == WIFI_EVENT_STA_DISCONNECTED) {
        if (up) {
            link_log("warn", "wifi: disconnected");
        }
        up = false;
        int delay = retries < 5 ? 1000 : retries < 20 ? 5000 : 30000;
        retries++;
        xTaskCreate(retry_task, "wifi_retry", 2048, (void *)(intptr_t)delay, 3, NULL);
    } else if (base == IP_EVENT && id == IP_EVENT_STA_GOT_IP) {
        ip_event_got_ip_t *e = data;
        up = true;
        retries = 0;
        link_log("info", "wifi: connected, ip " IPSTR, IP2STR(&e->ip_info.ip));
    }
}

void wifi_start(void) {
    const cJSON *w = cfg_section("wifi");
    const char *ssid = cfg_str(w, "ssid", "");
    if (!*ssid) {
        ESP_LOGW(TAG, "no wifi.ssid configured");
        return;
    }
    esp_netif_init();
    esp_event_loop_create_default();
    netif = esp_netif_create_default_wifi_sta();
    esp_netif_set_hostname(netif, "deck-bridge");
    wifi_init_config_t ic = WIFI_INIT_CONFIG_DEFAULT();
    esp_wifi_init(&ic);
    esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, on_event, NULL);
    esp_event_handler_register(IP_EVENT, IP_EVENT_STA_GOT_IP, on_event, NULL);
    wifi_config_t c = {0};
    strncpy((char *)c.sta.ssid, ssid, sizeof c.sta.ssid - 1);
    strncpy((char *)c.sta.password, cfg_str(w, "password", ""), sizeof c.sta.password - 1);
    c.sta.threshold.authmode = *c.sta.password ? WIFI_AUTH_WPA2_PSK : WIFI_AUTH_OPEN;
    esp_wifi_set_mode(WIFI_MODE_STA);
    esp_wifi_set_config(WIFI_IF_STA, &c);
    esp_wifi_set_ps(WIFI_PS_NONE);  // lowest latency for key presses
    esp_wifi_start();
}

bool wifi_up(void) {
    return up;
}

cJSON *wifi_status(void) {
    cJSON *o = cJSON_CreateObject();
    cJSON_AddStringToObject(o, "ssid", cfg_str(cfg_section("wifi"), "ssid", ""));
    cJSON_AddBoolToObject(o, "connected", up);
    if (up) {
        wifi_ap_record_t ap;
        if (esp_wifi_sta_get_ap_info(&ap) == ESP_OK) {
            cJSON_AddNumberToObject(o, "rssi", ap.rssi);
        }
        esp_netif_ip_info_t ip;
        if (netif && esp_netif_get_ip_info(netif, &ip) == ESP_OK) {
            char s[16];
            snprintf(s, sizeof s, IPSTR, IP2STR(&ip.ip));
            cJSON_AddStringToObject(o, "ip", s);
        }
    }
    return o;
}
