// MQTT: publish from keys, subscriptions become deck states.
//   config "mqtt": {"uri": "mqtt://broker:1883", "username": "...", "password": "...",
//                   "subscriptions": {"home/door/state": "door", "sensors/+/temp": "temp"}}
//   op "pub" {topic, payload, retain, qos}
//   states mqtt:<name> = last payload of the subscribed topic (wildcards: the last match wins)
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "esp_crt_bundle.h"
#include "mqtt_client.h"

static esp_mqtt_client_handle_t client;
static volatile bool connected;

static bool enabled(const plugin_t *self) {
    return *cfg_str(cfg_section("mqtt"), "uri", "");
}

static bool ready(const plugin_t *self) {
    return connected;
}

// MQTT topic filter match with + and #.
bool mqtt_topic_matches(const char *filter, const char *topic, int tlen) {
    const char *t = topic, *end = topic + tlen;
    while (*filter) {
        if (*filter == '#') return true;
        if (*filter == '+') {
            while (t < end && *t != '/') t++;
            filter++;
        } else {
            if (t >= end || *filter != *t) return false;
            filter++;
            t++;
        }
    }
    return t == end;
}

static void subscribe_topic(const char *topic) {
    esp_mqtt_client_subscribe(client, topic, 0);
}

bool mqtt_publish(const char *topic, const char *payload, int qos, bool retain) {
    if (!client || !connected || !topic || !*topic) return false;
    return esp_mqtt_client_publish(client, topic, payload, 0, qos < 0 || qos > 2 ? 0 : qos, retain) >= 0;
}

static void on_mqtt(void *arg, esp_event_base_t base, int32_t id, void *data) {
    esp_mqtt_event_handle_t e = data;
    const cJSON *subs = cJSON_GetObjectItem(cfg_section("mqtt"), "subscriptions");
    switch ((esp_mqtt_event_id_t)id) {
    case MQTT_EVENT_CONNECTED: {
        connected = true;
        const cJSON *s;
        cJSON_ArrayForEach(s, subs) esp_mqtt_client_subscribe(client, s->string, 0);
        decl_mqtt_subscribe(subscribe_topic);
        link_log("info", "mqtt: connected, %d subscriptions", cJSON_GetArraySize(subs));
        break;
    }
    case MQTT_EVENT_DISCONNECTED:
        if (connected) link_log("warn", "mqtt: disconnected");
        connected = false;
        break;
    case MQTT_EVENT_DATA: {
        if (e->current_data_offset != 0 || e->data_len != e->total_data_len) break;  // only whole small messages
        char val[1024];
        int n = e->data_len < (int)sizeof val - 1 ? e->data_len : (int)sizeof val - 1;
        memcpy(val, e->data, n);
        val[n] = 0;
        const cJSON *s;
        cJSON_ArrayForEach(s, subs) {
            if (cJSON_IsString(s) && mqtt_topic_matches(s->string, e->topic, e->topic_len)) {
                char name[96];
                snprintf(name, sizeof name, "mqtt:%s", s->valuestring);
                store_set(name, val);
            }
        }
        decl_mqtt_message(e->topic, e->topic_len, val);
        break;
    }
    default:
        break;
    }
}

static void start(const plugin_t *self) {
    const cJSON *c = cfg_section("mqtt");
    esp_mqtt_client_config_t mc = {
        .broker.address.uri = cfg_str(c, "uri", ""),
        .broker.verification.crt_bundle_attach = esp_crt_bundle_attach,
        .credentials.username = cfg_str(c, "username", NULL),
        .credentials.authentication.password = cfg_str(c, "password", NULL),
        .credentials.client_id = "deck-bridge",
    };
    client = esp_mqtt_client_init(&mc);
    esp_mqtt_client_register_event(client, ESP_EVENT_ANY_ID, on_mqtt, NULL);
    esp_mqtt_client_start(client);
}

static bool run(const plugin_t *self, const char *op, const cJSON *args, const char *event, int delta, char *err, size_t n) {
    if (strcmp(op, "pub")) {
        snprintf(err, n, "mqtt: unknown op %s", op);
        return false;
    }
    const char *topic = cJSON_GetStringValue(cJSON_GetObjectItem(args, "topic"));
    if (!topic) {
        snprintf(err, n, "pub: missing topic");
        return false;
    }
    char payload[512];
    json_value_text(cJSON_GetObjectItem(args, "payload"), payload, sizeof payload);
    int qos = (int)cJSON_GetNumberValue(cJSON_GetObjectItem(args, "qos"));
    if (qos < 0 || qos > 2) qos = 0;
    int r = esp_mqtt_client_publish(client, topic, payload, 0, qos, cJSON_IsTrue(cJSON_GetObjectItem(args, "retain")));
    if (r < 0) {
        snprintf(err, n, "pub %s failed", topic);
        return false;
    }
    return true;
}

const plugin_t plugin_mqtt = {
    .name = "mqtt",
    .prefix = "mqtt",
    .enabled = enabled,
    .start = start,
    .ready = ready,
    .run = run,
};
