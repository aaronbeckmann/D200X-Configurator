// Bridge config: one JSON document in NVS ("bridge"/"config"). Set over the deck (0x0320) or the
// USB console; applied by restarting. Secrets stay here, never on the deck.
//
// {
//   "wifi": {"ssid": "...", "password": "..."},
//   "homeassistant": {"url": "http://homeassistant.local:8123", "token": "..."},
//   "mqtt": {"uri": "mqtt://broker:1883", "username": "...", "password": "...",
//            "subscriptions": {"home/door": "door"}},            -> state mqtt:door
//   "net": {"polls": [{"state": "btc", "url": "https://...", "pointer": "/price", "every_s": 60}]},
//   "connections": {"nas": {"url": "http://192.168.1.5:5000", "headers": {"Authorization": "..."}}},
//   "obs": {"url": "ws://192.168.1.10:4455", "password": "..."},
//   "spotify": {"client_id": "...", "refresh_token": "..."},
//   "plugins": {"wled": {"url": "...", "ops": {...}, "states": {...}}}   (esp32-bridge/docs/PLUGINS.md)
// }
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "esp_log.h"
#include "nvs.h"
#include "nvs_flash.h"

static const char *TAG = "cfg";
static cJSON *root;

static char *load_blob(void) {
    nvs_handle_t h;
    if (nvs_open("bridge", NVS_READONLY, &h) != ESP_OK) {
        return NULL;
    }
    // a blob, not a string: NVS strings stop at 4000 bytes, plugin definitions need more
    size_t len = 0;
    char *s = NULL;
    if (nvs_get_blob(h, "cfg", NULL, &len) == ESP_OK && len > 0) {
        s = malloc(len + 1);
        if (s && nvs_get_blob(h, "cfg", s, &len) == ESP_OK) {
            s[len] = 0;
        } else {
            free(s);
            s = NULL;
        }
    } else if (nvs_get_str(h, "config", NULL, &len) == ESP_OK && len > 0) {
        s = malloc(len);  // written by firmware 0.1.0
        if (s && nvs_get_str(h, "config", s, &len) != ESP_OK) {
            free(s);
            s = NULL;
        }
    }
    nvs_close(h);
    return s;
}

static bool store_blob(const char *s) {
    nvs_handle_t h;
    if (nvs_open("bridge", NVS_READWRITE, &h) != ESP_OK) {
        return false;
    }
    bool ok = nvs_set_blob(h, "cfg", s, strlen(s)) == ESP_OK && nvs_commit(h) == ESP_OK;
    if (ok) nvs_erase_key(h, "config");
    nvs_commit(h);
    nvs_close(h);
    return ok;
}

void cfg_init(void) {
    esp_err_t e = nvs_flash_init();
    if (e == ESP_ERR_NVS_NO_FREE_PAGES || e == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        nvs_flash_erase();
        nvs_flash_init();
    }
    char *s = load_blob();
    root = s ? cJSON_Parse(s) : NULL;
    free(s);
    if (!cJSON_IsObject(root)) {
        cJSON_Delete(root);
        root = cJSON_CreateObject();
        ESP_LOGW(TAG, "no config stored: set one with 'config {...}' on the USB console or d200x-tool bridge-config");
    }
}

const cJSON *cfg_section(const char *name) {
    const cJSON *s = cJSON_GetObjectItemCaseSensitive(root, name);
    return cJSON_IsObject(s) ? s : NULL;
}

const char *cfg_str(const cJSON *obj, const char *key, const char *def) {
    const cJSON *v = cJSON_GetObjectItemCaseSensitive(obj, key);
    return cJSON_IsString(v) ? v->valuestring : def;
}

static void restore_secrets(cJSON *now, const cJSON *old) {
    cJSON *k;
    cJSON_ArrayForEach(k, now) {
        const cJSON *o = cJSON_IsArray(now) ? NULL : cJSON_GetObjectItemCaseSensitive(old, k->string);
        if (cJSON_IsString(k) && !strcmp(k->valuestring, "***") && cJSON_IsString(o)) {
            cJSON_SetValuestring(k, o->valuestring);
        } else if (cJSON_IsObject(k) && o) {
            restore_secrets(k, o);
        }
    }
}

bool cfg_save(const char *json, char *err, size_t errlen) {
    cJSON *c = cJSON_Parse(json);
    if (!cJSON_IsObject(c)) {
        snprintf(err, errlen, "config is not a JSON object");
        cJSON_Delete(c);
        return false;
    }
    // secrets left as "***" (from a status read-back) keep their stored value
    restore_secrets(c, root);
    char *s = cJSON_PrintUnformatted(c);
    cJSON_Delete(c);
    bool ok = s && store_blob(s);
    free(s);
    if (!ok) {
        snprintf(err, errlen, "NVS write failed");
    }
    return ok;
}

void cfg_update_string(const char *section, const char *key, const char *value) {
    cJSON *sec = cJSON_GetObjectItemCaseSensitive(root, section);
    if (!cJSON_IsObject(sec)) {
        return;
    }
    cJSON_DeleteItemFromObjectCaseSensitive(sec, key);
    cJSON_AddStringToObject(sec, key, value);
    char *s = cJSON_PrintUnformatted(root);
    if (s) {
        store_blob(s);
        free(s);
    }
}

static bool secret_key(const char *k) {
    return strstr(k, "password") || strstr(k, "token") || strstr(k, "secret") || !strcmp(k, "Authorization");
}

static void redact(cJSON *o) {
    cJSON *k;
    cJSON_ArrayForEach(k, o) {
        if (cJSON_IsString(k) && k->string && secret_key(k->string)) {
            cJSON_SetValuestring(k, "***");
        } else if (cJSON_IsObject(k) || cJSON_IsArray(k)) {
            redact(k);
        }
    }
}

cJSON *cfg_redacted(void) {
    cJSON *c = cJSON_Duplicate(root, true);
    redact(c);
    return c;
}
