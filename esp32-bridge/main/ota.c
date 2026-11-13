// Over-the-air update from an http(s) URL into the other app slot, then restart.
// Rollback: a new image boots "pending verify"; it confirms itself once Wi-Fi is up (or after
// 60 s without a crash). If it crashes or resets before that, the bootloader returns to the old one.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "esp_crt_bundle.h"
#include "esp_https_ota.h"
#include "esp_ota_ops.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static volatile bool running;

static void ota_task(void *arg) {
    char *url = arg;
    link_log("info", "ota: downloading %s", url);
    esp_http_client_config_t http = {
        .url = url,
        .timeout_ms = 15000,
        .crt_bundle_attach = esp_crt_bundle_attach,
        .keep_alive_enable = true,
    };
    esp_https_ota_config_t c = {.http_config = &http};
    esp_https_ota_handle_t h = NULL;
    esp_err_t e = esp_https_ota_begin(&c, &h);
    int last_pct = -1;
    while (e == ESP_OK) {
        e = esp_https_ota_perform(h);
        if (e != ESP_ERR_HTTPS_OTA_IN_PROGRESS) break;
        int size = esp_https_ota_get_image_size(h);
        int pct = size > 0 ? (int)(100LL * esp_https_ota_get_image_len_read(h) / size) : 0;
        if (pct / 20 != last_pct / 20) {
            last_pct = pct;
            link_log("info", "ota: %d%%", pct);
        }
        e = ESP_OK;
    }
    if (e == ESP_OK && h && !esp_https_ota_is_complete_data_received(h)) e = ESP_FAIL;
    if (e == ESP_OK) {
        e = esp_https_ota_finish(h);  // validates the image and selects it for the next boot
        h = NULL;
    }
    if (h) esp_https_ota_abort(h);
    if (e == ESP_OK) {
        link_log("info", "ota: installed, restarting");
        link_schedule_restart(1000);
    } else {
        link_log("error", "ota: failed: %s", esp_err_to_name(e));
        running = false;
    }
    free(url);
    vTaskDelete(NULL);
}

bool ota_start(const char *url, char *err, size_t errlen) {
    if (running) {
        snprintf(err, errlen, "an update is already running");
        return false;
    }
    if (!url || (strncmp(url, "http://", 7) && strncmp(url, "https://", 8))) {
        snprintf(err, errlen, "need an http(s) URL");
        return false;
    }
    if (!wifi_up()) {
        snprintf(err, errlen, "wifi not connected");
        return false;
    }
    running = true;
    xTaskCreate(ota_task, "ota", 8192, strdup(url), 4, NULL);
    return true;
}

static void confirm_task(void *arg) {
    const esp_partition_t *p = esp_ota_get_running_partition();
    esp_ota_img_states_t st;
    if (esp_ota_get_state_partition(p, &st) == ESP_OK && st == ESP_OTA_IMG_PENDING_VERIFY) {
        int64_t t0 = now_ms();
        while (!wifi_up() && now_ms() - t0 < 60000) vTaskDelay(pdMS_TO_TICKS(500));
        esp_ota_mark_app_valid_cancel_rollback();
        link_log("info", "ota: new firmware %s confirmed", BRIDGE_VERSION);
    }
    vTaskDelete(NULL);
}

void ota_confirm_task_start(void) {
    xTaskCreate(confirm_task, "ota_confirm", 3072, NULL, 2, NULL);
}
