// Home Assistant over its WebSocket API. Same ops and state names as the PC agent
// (d200x-agent/src/ha.rs): ha:<entity>, ha:<entity>:text, ha:<entity>:name, ha:<entity>:attr:<a>.
// Unlike the PC agent it never loads all entities: `subscribe_entities` is filtered to the
// entities the deck shows (hello's wanted list), and refreshed when that list changes.
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "esp_crt_bundle.h"
#include "esp_heap_caps.h"
#include "esp_websocket_client.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"

#define MSG_MAX (256 * 1024)

static esp_websocket_client_handle_t ws;
static SemaphoreHandle_t lock, call_done;
static cJSON *cache;  // entity_id -> {"s": state, "a": {attributes}}
static volatile bool authed, need_auth, need_sub;
static int next_id = 1, sub_id;
static uint32_t sub_ver;
static volatile int call_id;
static bool call_ok;
static char call_err[120];
static char *msg;
static int msg_cap;
static char base_url[160], token[300];

static bool enabled(const plugin_t *self) {
    const cJSON *s = cfg_section("homeassistant");
    return s && *cfg_str(s, "url", "") && *cfg_str(s, "token", "");
}

static bool ready(const plugin_t *self) {
    return authed;
}

static int take_id(void) {
    xSemaphoreTake(lock, portMAX_DELAY);
    int id = next_id++;
    xSemaphoreGive(lock);
    return id;
}

static void ws_send(cJSON *o) {
    char *s = cJSON_PrintUnformatted(o);
    if (s) {
        esp_websocket_client_send_text(ws, s, strlen(s), pdMS_TO_TICKS(3000));
        free(s);
    }
}

// ---- states ---------------------------------------------------------------------------------

// Push every wanted state derived from one entity (call with lock held).
static void publish_locked(const char *id) {
    const cJSON *e = cJSON_GetObjectItemCaseSensitive(cache, id);
    if (!e) return;
    const char *state = cJSON_GetStringValue(cJSON_GetObjectItem(e, "s")) ?: "";
    const cJSON *attrs = cJSON_GetObjectItem(e, "a");
    cJSON *names = store_wanted_with_prefix("ha:");
    size_t idlen = strlen(id);
    const cJSON *n;
    cJSON_ArrayForEach(n, names) {
        const char *rest = n->valuestring + 3;
        if (strncmp(rest, id, idlen) || (rest[idlen] && rest[idlen] != ':')) continue;
        const char *kind = rest[idlen] == ':' ? rest + idlen + 1 : NULL;
        char val[256];
        if (!kind) {
            snprintf(val, sizeof val, "%s", state);
        } else if (!strcmp(kind, "text")) {
            const char *unit = cJSON_GetStringValue(cJSON_GetObjectItem(attrs, "unit_of_measurement"));
            snprintf(val, sizeof val, unit && *unit ? "%s %s" : "%s", state, unit);
        } else if (!strcmp(kind, "name")) {
            snprintf(val, sizeof val, "%s", cJSON_GetStringValue(cJSON_GetObjectItem(attrs, "friendly_name")) ?: id);
        } else if (!strncmp(kind, "attr:", 5)) {
            json_value_text(cJSON_GetObjectItemCaseSensitive(attrs, kind + 5), val, sizeof val);
        } else {
            continue;
        }
        store_set(n->valuestring, val);
    }
    cJSON_Delete(names);
}

// subscribe_entities event: "a" = full states, "c" = changes ("+" set, "-" removed attributes),
// "r" = removed entities.
static void apply_event(const cJSON *ev) {
    xSemaphoreTake(lock, portMAX_DELAY);
    const cJSON *x;
    cJSON_ArrayForEach(x, cJSON_GetObjectItem(ev, "a")) {
        cJSON *e = cJSON_CreateObject();
        cJSON_AddItemToObject(e, "s", cJSON_Duplicate(cJSON_GetObjectItem(x, "s"), true));
        cJSON_AddItemToObject(e, "a", cJSON_Duplicate(cJSON_GetObjectItem(x, "a"), true));
        cJSON_DeleteItemFromObjectCaseSensitive(cache, x->string);
        cJSON_AddItemToObject(cache, x->string, e);
        publish_locked(x->string);
    }
    cJSON_ArrayForEach(x, cJSON_GetObjectItem(ev, "c")) {
        cJSON *e = cJSON_GetObjectItemCaseSensitive(cache, x->string);
        if (!e) continue;
        const cJSON *plus = cJSON_GetObjectItem(x, "+");
        const cJSON *s = cJSON_GetObjectItem(plus, "s");
        if (s) cJSON_ReplaceItemInObject(e, "s", cJSON_Duplicate(s, true));
        cJSON *attrs = cJSON_GetObjectItem(e, "a");
        const cJSON *a;
        cJSON_ArrayForEach(a, cJSON_GetObjectItem(plus, "a")) {
            cJSON_DeleteItemFromObjectCaseSensitive(attrs, a->string);
            cJSON_AddItemToObject(attrs, a->string, cJSON_Duplicate(a, true));
        }
        cJSON_ArrayForEach(a, cJSON_GetObjectItem(cJSON_GetObjectItem(x, "-"), "a")) {
            if (cJSON_IsString(a)) cJSON_DeleteItemFromObjectCaseSensitive(attrs, a->valuestring);
        }
        publish_locked(x->string);
    }
    cJSON_ArrayForEach(x, cJSON_GetObjectItem(ev, "r")) {
        if (cJSON_IsString(x)) cJSON_DeleteItemFromObjectCaseSensitive(cache, x->valuestring);
    }
    xSemaphoreGive(lock);
}

static void subscribe(void) {
    sub_ver = store_wanted_version();
    cJSON *names = store_wanted_with_prefix("ha:");
    cJSON *ids = cJSON_CreateArray();
    const cJSON *n;
    cJSON_ArrayForEach(n, names) {
        char id[128];
        snprintf(id, sizeof id, "%s", n->valuestring + 3);
        char *colon = strchr(id, ':');
        if (colon) *colon = 0;
        bool have = false;
        const cJSON *i;
        cJSON_ArrayForEach(i, ids) have |= !strcmp(i->valuestring, id);
        if (!have) cJSON_AddItemToArray(ids, cJSON_CreateString(id));
    }
    cJSON_Delete(names);
    if (sub_id) {
        cJSON *u = cJSON_CreateObject();
        cJSON_AddNumberToObject(u, "id", take_id());
        cJSON_AddStringToObject(u, "type", "unsubscribe_events");
        cJSON_AddNumberToObject(u, "subscription", sub_id);
        ws_send(u);
        cJSON_Delete(u);
        sub_id = 0;
    }
    xSemaphoreTake(lock, portMAX_DELAY);
    cJSON_Delete(cache);
    cache = cJSON_CreateObject();
    xSemaphoreGive(lock);
    int count = cJSON_GetArraySize(ids);
    if (count) {
        sub_id = take_id();
        cJSON *o = cJSON_CreateObject();
        cJSON_AddNumberToObject(o, "id", sub_id);
        cJSON_AddStringToObject(o, "type", "subscribe_entities");
        cJSON_AddItemToObject(o, "entity_ids", ids);
        ws_send(o);
        cJSON_Delete(o);
    } else {
        cJSON_Delete(ids);
    }
    link_log("info", "ha: watching %d entities", count);
}

// ---- WebSocket ------------------------------------------------------------------------------

static void on_message(const char *text) {
    cJSON *j = cJSON_Parse(text);
    const char *type = cJSON_GetStringValue(cJSON_GetObjectItem(j, "type")) ?: "";
    int id = (int)cJSON_GetNumberValue(cJSON_GetObjectItem(j, "id"));
    if (!strcmp(type, "auth_required")) {
        need_auth = true;
    } else if (!strcmp(type, "auth_ok")) {
        authed = true;
        need_sub = true;
        link_log("info", "ha: connected (%s)", cJSON_GetStringValue(cJSON_GetObjectItem(j, "ha_version")) ?: "?");
    } else if (!strcmp(type, "auth_invalid")) {
        link_log("error", "ha: token rejected: %s", cJSON_GetStringValue(cJSON_GetObjectItem(j, "message")) ?: "");
    } else if (!strcmp(type, "result") && id && id == call_id) {
        call_ok = cJSON_IsTrue(cJSON_GetObjectItem(j, "success"));
        snprintf(call_err, sizeof call_err, "%s", cJSON_GetStringValue(cJSON_GetObjectItem(cJSON_GetObjectItem(j, "error"), "message")) ?: "failed");
        xSemaphoreGive(call_done);
    } else if (!strcmp(type, "event") && id == sub_id) {
        apply_event(cJSON_GetObjectItem(j, "event"));
    }
    cJSON_Delete(j);
}

static void on_ws(void *arg, esp_event_base_t base, int32_t event, void *data) {
    esp_websocket_event_data_t *d = data;
    switch (event) {
    case WEBSOCKET_EVENT_DISCONNECTED:
    case WEBSOCKET_EVENT_CLOSED:
    case WEBSOCKET_EVENT_ERROR:
        if (authed) link_log("warn", "ha: disconnected");
        authed = false;
        sub_id = 0;
        if (call_id) {
            call_ok = false;
            snprintf(call_err, sizeof call_err, "disconnected");
            xSemaphoreGive(call_done);
        }
        break;
    case WEBSOCKET_EVENT_DATA:
        if (d->op_code != 1 && d->op_code != 0) break;  // text and its continuations
        if (d->payload_len <= 0 || d->payload_len > MSG_MAX) break;
        if (d->payload_len + 1 > msg_cap) {
            char *n = heap_caps_realloc(msg, d->payload_len + 1, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
            if (!n) break;
            msg = n;
            msg_cap = d->payload_len + 1;
        }
        memcpy(msg + d->payload_offset, d->data_ptr, d->data_len);
        if (d->payload_offset + d->data_len >= d->payload_len) {
            msg[d->payload_len] = 0;
            on_message(msg);
        }
        break;
    default:
        break;
    }
}

static void ha_task(void *arg) {
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(100));
        if (!esp_websocket_client_is_connected(ws)) continue;
        if (need_auth) {
            need_auth = false;
            cJSON *o = cJSON_CreateObject();
            cJSON_AddStringToObject(o, "type", "auth");
            cJSON_AddStringToObject(o, "access_token", token);
            ws_send(o);
            cJSON_Delete(o);
        }
        if (authed && (need_sub || store_wanted_version() != sub_ver)) {
            need_sub = false;
            subscribe();
        }
    }
}

static void start(const plugin_t *self) {
    const cJSON *c = cfg_section("homeassistant");
    snprintf(base_url, sizeof base_url, "%s", cfg_str(c, "url", ""));
    size_t l = strlen(base_url);
    while (l && base_url[l - 1] == '/') base_url[--l] = 0;
    snprintf(token, sizeof token, "%s", cfg_str(c, "token", ""));
    lock = xSemaphoreCreateMutex();
    call_done = xSemaphoreCreateBinary();
    cache = cJSON_CreateObject();
    // http(s)://host:port -> ws(s)://host:port/api/websocket
    char uri[200];
    bool tls = !strncmp(base_url, "https://", 8);
    snprintf(uri, sizeof uri, "%s%s/api/websocket", tls ? "wss://" : "ws://", base_url + (tls ? 8 : strncmp(base_url, "http://", 7) ? 0 : 7));
    esp_websocket_client_config_t wc = {
        .uri = uri,
        .buffer_size = 4096,
        .task_stack = 8192,
        .reconnect_timeout_ms = 5000,
        .network_timeout_ms = 10000,
        .crt_bundle_attach = esp_crt_bundle_attach,
    };
    ws = esp_websocket_client_init(&wc);
    esp_websocket_register_events(ws, WEBSOCKET_EVENT_ANY, on_ws, NULL);
    esp_websocket_client_start(ws);
    xTaskCreate(ha_task, "ha", 6144, NULL, 4, NULL);
}

// ---- service calls ----------------------------------------------------------------------------

static bool call(const char *domain, const char *service, cJSON *data, char *err, size_t n) {
    int id = take_id();
    cJSON *o = cJSON_CreateObject();
    cJSON_AddNumberToObject(o, "id", id);
    cJSON_AddStringToObject(o, "type", "call_service");
    cJSON_AddStringToObject(o, "domain", domain);
    cJSON_AddStringToObject(o, "service", service);
    cJSON_AddItemToObject(o, "service_data", data);
    xSemaphoreTake(call_done, 0);
    call_id = id;
    ws_send(o);
    cJSON_Delete(o);
    bool ok = false;
    if (xSemaphoreTake(call_done, pdMS_TO_TICKS(8000)) != pdTRUE) {
        snprintf(err, n, "%s.%s: no answer", domain, service);
    } else if (!(ok = call_ok)) {
        snprintf(err, n, "%s.%s: %s", domain, service, call_err);
    }
    call_id = 0;
    return ok;
}

static cJSON *with_entity(const char *entity) {
    cJSON *d = cJSON_CreateObject();
    cJSON_AddStringToObject(d, "entity_id", entity);
    return d;
}

// An entity's current state: from the subscription if the deck shows it, else over REST.
static cJSON *entity_now(const char *id) {
    cJSON *e = NULL;
    xSemaphoreTake(lock, portMAX_DELAY);
    const cJSON *c = cJSON_GetObjectItemCaseSensitive(cache, id);
    if (c) {
        e = cJSON_CreateObject();
        cJSON_AddItemToObject(e, "state", cJSON_Duplicate(cJSON_GetObjectItem(c, "s"), true));
        cJSON_AddItemToObject(e, "attributes", cJSON_Duplicate(cJSON_GetObjectItem(c, "a"), true));
    }
    xSemaphoreGive(lock);
    if (e) return e;
    char url[256], auth[320];
    snprintf(url, sizeof url, "%s/api/states/%s", base_url, id);
    snprintf(auth, sizeof auth, "Bearer %s", token);
    cJSON *h = cJSON_CreateObject();
    cJSON_AddStringToObject(h, "Authorization", auth);
    char *body = NULL;
    int st = http_request("GET", url, h, NULL, NULL, &body);
    cJSON_Delete(h);
    if (st == 200) e = cJSON_Parse(body);
    free(body);
    return e;
}

static double attr_num(const cJSON *e, const char *a, double def) {
    const cJSON *v = cJSON_GetObjectItem(cJSON_GetObjectItem(e, "attributes"), a);
    return cJSON_IsNumber(v) ? v->valuedouble : def;
}

static double arg_num(const cJSON *args, const char *k, double def) {
    const cJSON *v = cJSON_GetObjectItem(args, k);
    return cJSON_IsNumber(v) ? v->valuedouble : def;
}

static bool run(const plugin_t *self, const char *op, const cJSON *args, const char *event, int delta, char *err, size_t n) {
    const char *entity = cJSON_GetStringValue(cJSON_GetObjectItem(args, "entity_id"));
    bool press = !strcmp(event, "press");
    char domain[48] = "homeassistant";
    if (entity) {
        snprintf(domain, sizeof domain, "%s", entity);
        char *dot = strchr(domain, '.');
        if (dot) *dot = 0;
    }
    if (!strcmp(op, "call_service")) {
        const char *d = cJSON_GetStringValue(cJSON_GetObjectItem(args, "domain"));
        const char *s = cJSON_GetStringValue(cJSON_GetObjectItem(args, "service"));
        if (!d || !s) {
            snprintf(err, n, "call_service: missing domain or service");
            return false;
        }
        cJSON *data = cJSON_Duplicate(cJSON_GetObjectItem(args, "data"), true) ?: cJSON_CreateObject();
        if (entity) cJSON_AddStringToObject(data, "entity_id", entity);
        return call(d, s, data, err, n);
    }
    if (!entity) {
        snprintf(err, n, "%s: missing entity_id", op);
        return false;
    }
    if (!strcmp(op, "toggle") || !strcmp(op, "turn_on") || !strcmp(op, "turn_off")) {
        cJSON *data = with_entity(entity);
        const cJSON *x;
        cJSON_ArrayForEach(x, cJSON_GetObjectItem(args, "data")) {
            cJSON_AddItemToObject(data, x->string, cJSON_Duplicate(x, true));
        }
        bool own = !strcmp(domain, "input_boolean") || !strcmp(domain, "automation");
        return call(own ? domain : "homeassistant", op, data, err, n);
    }
    if (!strcmp(op, "trigger")) {
        cJSON *data = with_entity(entity);
        const cJSON *skip = cJSON_GetObjectItem(args, "skip_condition");
        cJSON_AddBoolToObject(data, "skip_condition", skip ? cJSON_IsTrue(skip) : true);
        return call("automation", "trigger", data, err, n);
    }
    if (!strcmp(op, "scene")) return call("scene", "turn_on", with_entity(entity), err, n);
    if (!strcmp(op, "script")) return call("script", "turn_on", with_entity(entity), err, n);
    if (!strcmp(op, "media")) {
        return call("media_player", cJSON_GetStringValue(cJSON_GetObjectItem(args, "command")) ?: "media_play_pause", with_entity(entity), err, n);
    }
    if (!strcmp(op, "brightness")) {
        if (press) return call("light", "toggle", with_entity(entity), err, n);
        int pct = (int)arg_num(args, "step", 10) * delta;
        cJSON *data = with_entity(entity);
        cJSON_AddNumberToObject(data, "brightness_step_pct", pct > 100 ? 100 : pct < -100 ? -100 : pct);
        return call("light", "turn_on", data, err, n);
    }
    if (!strcmp(op, "volume")) {
        if (press) return call("media_player", "media_play_pause", with_entity(entity), err, n);
        cJSON *e = entity_now(entity);
        double cur = attr_num(e, "volume_level", -1);
        cJSON_Delete(e);
        if (cur < 0) return call("media_player", delta > 0 ? "volume_up" : "volume_down", with_entity(entity), err, n);
        double v = cur + arg_num(args, "step", 5) / 100.0 * delta;
        cJSON *data = with_entity(entity);
        cJSON_AddNumberToObject(data, "volume_level", v < 0 ? 0 : v > 1 ? 1 : v);
        return call("media_player", "volume_set", data, err, n);
    }
    if (!strcmp(op, "temperature")) {
        cJSON *e = entity_now(entity);
        double cur = attr_num(e, "temperature", NAN);
        cJSON_Delete(e);
        if (isnan(cur)) {
            snprintf(err, n, "temperature: no current target temperature");
            return false;
        }
        cJSON *data = with_entity(entity);
        cJSON_AddNumberToObject(data, "temperature", cur + arg_num(args, "step", 0.5) * delta);
        return call("climate", "set_temperature", data, err, n);
    }
    if (!strcmp(op, "number")) {
        cJSON *e = entity_now(entity);
        if (!e) {
            snprintf(err, n, "number: unknown entity");
            return false;
        }
        double cur = atof(cJSON_GetStringValue(cJSON_GetObjectItem(e, "state")) ?: "0");
        double step = arg_num(args, "step", attr_num(e, "step", 1));
        cJSON_Delete(e);
        cJSON *data = with_entity(entity);
        cJSON_AddNumberToObject(data, "value", cur + step * delta);
        return call(domain, "set_value", data, err, n);
    }
    if (!strcmp(op, "select_next")) {
        cJSON *data = with_entity(entity);
        cJSON_AddBoolToObject(data, "cycle", true);
        return call(domain, delta >= 0 ? "select_next" : "select_previous", data, err, n);
    }
    snprintf(err, n, "homeassistant: unknown op %s", op);
    return false;
}

const plugin_t plugin_homeassistant = {
    .name = "homeassistant",
    .prefix = "ha",
    .enabled = enabled,
    .start = start,
    .ready = ready,
    .run = run,
};
