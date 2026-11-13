// OBS Studio over obs-websocket v5 on the LAN. Same ops and states as the PC agent
// (d200x-agent/src/obs.rs).
//   config "obs": {"url": "ws://192.168.1.10:4455", "password": "..."}
//   states obs:connected, obs:scene, obs:recording, obs:streaming, obs:virtualcam, obs:mute:<input>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "esp_heap_caps.h"
#include "esp_websocket_client.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "psa/crypto.h"

#define MSG_MAX (64 * 1024)
// event subscriptions: Scenes | Inputs | Outputs | Ui
#define OBS_EVENTS (4 | 8 | 64 | 1024)

static esp_websocket_client_handle_t ws;
static volatile bool identified, need_identify, need_init;
static char challenge[96], salt[96];
static SemaphoreHandle_t req_done, req_lock;
static char req_id[16];
static int req_seq;
static bool req_ok;
static char req_err[120];
static cJSON *req_resp;
static char *msg;
static int msg_cap;
static uint32_t mute_ver;

static bool enabled(const plugin_t *self) {
    return *cfg_str(cfg_section("obs"), "url", "");
}

static bool ready(const plugin_t *self) {
    return identified;
}

static void ws_send(cJSON *o) {
    char *s = cJSON_PrintUnformatted(o);
    if (s) {
        esp_websocket_client_send_text(ws, s, strlen(s), pdMS_TO_TICKS(3000));
        free(s);
    }
}

static void bool_state(const char *name, const cJSON *v) {
    store_set(name, cJSON_IsTrue(v) ? "true" : "false");
}

static void sha256_b64(const char *a, const char *b, char *out, size_t n) {
    size_t la = strlen(a), lb = strlen(b);
    char *in = malloc(la + lb + 1);
    memcpy(in, a, la);
    memcpy(in + la, b, lb + 1);
    uint8_t h[32];
    size_t hl = 0;
    psa_hash_compute(PSA_ALG_SHA_256, (const uint8_t *)in, la + lb, h, sizeof h, &hl);
    free(in);
    base64_encode(h, hl, out, n);
}

// Synchronous request (op 6 -> op 7). *resp gets responseData (caller frees) when given.
static bool request(const char *type, cJSON *data, cJSON **resp, char *err, size_t n) {
    xSemaphoreTake(req_lock, portMAX_DELAY);
    snprintf(req_id, sizeof req_id, "r%d", ++req_seq);
    xSemaphoreTake(req_done, 0);
    cJSON *o = cJSON_CreateObject();
    cJSON_AddNumberToObject(o, "op", 6);
    cJSON *d = cJSON_AddObjectToObject(o, "d");
    cJSON_AddStringToObject(d, "requestType", type);
    cJSON_AddStringToObject(d, "requestId", req_id);
    cJSON_AddItemToObject(d, "requestData", data ? data : cJSON_CreateObject());
    ws_send(o);
    cJSON_Delete(o);
    bool ok = false;
    if (xSemaphoreTake(req_done, pdMS_TO_TICKS(5000)) != pdTRUE) {
        snprintf(err, n, "%s: no answer", type);
    } else if (!(ok = req_ok)) {
        snprintf(err, n, "%s: %s", type, req_err);
    }
    if (resp) {
        *resp = req_resp;
    } else {
        cJSON_Delete(req_resp);
    }
    req_resp = NULL;
    req_id[0] = 0;
    xSemaphoreGive(req_lock);
    return ok;
}

static void refresh_mutes(void) {
    mute_ver = store_wanted_version();
    cJSON *names = store_wanted_with_prefix("obs:mute:");
    const cJSON *n;
    cJSON_ArrayForEach(n, names) {
        cJSON *d = cJSON_CreateObject(), *r = NULL;
        cJSON_AddStringToObject(d, "inputName", n->valuestring + 9);
        char err[120];
        if (request("GetInputMute", d, &r, err, sizeof err)) bool_state(n->valuestring, cJSON_GetObjectItem(r, "inputMuted"));
        cJSON_Delete(r);
    }
    cJSON_Delete(names);
}

static void initial_states(void) {
    char err[120];
    cJSON *r = NULL;
    if (request("GetCurrentProgramScene", NULL, &r, err, sizeof err))
        store_set("obs:scene", cJSON_GetStringValue(cJSON_GetObjectItem(r, "currentProgramSceneName")) ?: "");
    cJSON_Delete(r);
    const char *outs[][2] = {{"GetRecordStatus", "obs:recording"}, {"GetStreamStatus", "obs:streaming"}, {"GetVirtualCamStatus", "obs:virtualcam"}};
    for (int i = 0; i < 3; i++) {
        r = NULL;
        if (request(outs[i][0], NULL, &r, err, sizeof err)) bool_state(outs[i][1], cJSON_GetObjectItem(r, "outputActive"));
        cJSON_Delete(r);
    }
    refresh_mutes();
}

static void on_message(const char *text) {
    cJSON *j = cJSON_Parse(text);
    int op = (int)cJSON_GetNumberValue(cJSON_GetObjectItem(j, "op"));
    const cJSON *d = cJSON_GetObjectItem(j, "d");
    if (op == 0) {  // Hello
        const cJSON *auth = cJSON_GetObjectItem(d, "authentication");
        snprintf(challenge, sizeof challenge, "%s", cJSON_GetStringValue(cJSON_GetObjectItem(auth, "challenge")) ?: "");
        snprintf(salt, sizeof salt, "%s", cJSON_GetStringValue(cJSON_GetObjectItem(auth, "salt")) ?: "");
        need_identify = true;
    } else if (op == 2) {  // Identified
        identified = true;
        need_init = true;
        store_set("obs:connected", "true");
        link_log("info", "obs: connected");
    } else if (op == 7) {  // RequestResponse
        const char *id = cJSON_GetStringValue(cJSON_GetObjectItem(d, "requestId"));
        if (id && req_id[0] && !strcmp(id, req_id)) {
            const cJSON *st = cJSON_GetObjectItem(d, "requestStatus");
            req_ok = cJSON_IsTrue(cJSON_GetObjectItem(st, "result"));
            snprintf(req_err, sizeof req_err, "%s", cJSON_GetStringValue(cJSON_GetObjectItem(st, "comment")) ?: "failed");
            req_resp = cJSON_Duplicate(cJSON_GetObjectItem(d, "responseData"), true);
            xSemaphoreGive(req_done);
        }
    } else if (op == 5) {  // Event
        const char *t = cJSON_GetStringValue(cJSON_GetObjectItem(d, "eventType")) ?: "";
        const cJSON *ed = cJSON_GetObjectItem(d, "eventData");
        if (!strcmp(t, "CurrentProgramSceneChanged")) store_set("obs:scene", cJSON_GetStringValue(cJSON_GetObjectItem(ed, "sceneName")) ?: "");
        else if (!strcmp(t, "RecordStateChanged")) bool_state("obs:recording", cJSON_GetObjectItem(ed, "outputActive"));
        else if (!strcmp(t, "StreamStateChanged")) bool_state("obs:streaming", cJSON_GetObjectItem(ed, "outputActive"));
        else if (!strcmp(t, "VirtualcamStateChanged")) bool_state("obs:virtualcam", cJSON_GetObjectItem(ed, "outputActive"));
        else if (!strcmp(t, "InputMuteStateChanged")) {
            char name[128];
            snprintf(name, sizeof name, "obs:mute:%s", cJSON_GetStringValue(cJSON_GetObjectItem(ed, "inputName")) ?: "");
            bool_state(name, cJSON_GetObjectItem(ed, "inputMuted"));
        }
    }
    cJSON_Delete(j);
}

static void on_ws(void *arg, esp_event_base_t base, int32_t event, void *data) {
    esp_websocket_event_data_t *d = data;
    switch (event) {
    case WEBSOCKET_EVENT_DISCONNECTED:
    case WEBSOCKET_EVENT_CLOSED:
    case WEBSOCKET_EVENT_ERROR:
        if (identified) link_log("warn", "obs: disconnected");
        identified = false;
        store_set("obs:connected", "false");
        if (req_id[0]) {
            req_ok = false;
            snprintf(req_err, sizeof req_err, "disconnected");
            xSemaphoreGive(req_done);
        }
        break;
    case WEBSOCKET_EVENT_DATA:
        if ((d->op_code != 1 && d->op_code != 0) || d->payload_len <= 0 || d->payload_len > MSG_MAX) break;
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

static void obs_task(void *arg) {
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(100));
        if (!esp_websocket_client_is_connected(ws)) continue;
        if (need_identify) {
            need_identify = false;
            cJSON *o = cJSON_CreateObject();
            cJSON_AddNumberToObject(o, "op", 1);
            cJSON *d = cJSON_AddObjectToObject(o, "d");
            cJSON_AddNumberToObject(d, "rpcVersion", 1);
            cJSON_AddNumberToObject(d, "eventSubscriptions", OBS_EVENTS);
            if (challenge[0]) {
                char secret[48], auth[48];
                sha256_b64(cfg_str(cfg_section("obs"), "password", ""), salt, secret, sizeof secret);
                sha256_b64(secret, challenge, auth, sizeof auth);
                cJSON_AddStringToObject(d, "authentication", auth);
            }
            ws_send(o);
            cJSON_Delete(o);
        }
        if (identified && need_init) {
            need_init = false;
            initial_states();
        } else if (identified && store_wanted_version() != mute_ver) {
            refresh_mutes();
        }
    }
}

static void start(const plugin_t *self) {
    psa_crypto_init();
    req_done = xSemaphoreCreateBinary();
    req_lock = xSemaphoreCreateMutex();
    store_set("obs:connected", "false");
    esp_websocket_client_config_t wc = {
        .uri = cfg_str(cfg_section("obs"), "url", ""),
        .buffer_size = 4096,
        .task_stack = 6144,
        .reconnect_timeout_ms = 5000,
        .network_timeout_ms = 10000,
    };
    ws = esp_websocket_client_init(&wc);
    esp_websocket_register_events(ws, WEBSOCKET_EVENT_ANY, on_ws, NULL);
    esp_websocket_client_start(ws);
    xTaskCreate(obs_task, "obs", 6144, NULL, 4, NULL);
}

static cJSON *one(const char *k, const char *v) {
    cJSON *d = cJSON_CreateObject();
    cJSON_AddStringToObject(d, k, v);
    return d;
}

static bool run(const plugin_t *self, const char *op, const cJSON *args, const char *event, int delta, char *err, size_t n) {
    const char *name = cJSON_GetStringValue(cJSON_GetObjectItem(args, "name"));
    const char *input = cJSON_GetStringValue(cJSON_GetObjectItem(args, "input"));
    static const char *simple[][2] = {
        {"record_toggle", "ToggleRecord"}, {"record_start", "StartRecord"}, {"record_stop", "StopRecord"},
        {"stream_toggle", "ToggleStream"}, {"stream_start", "StartStream"}, {"stream_stop", "StopStream"},
        {"virtualcam_toggle", "ToggleVirtualCam"}, {"replay_save", "SaveReplayBuffer"},
        {"studio_toggle", "ToggleStudioMode"}, {"transition", "TriggerStudioModeTransition"},
    };
    for (size_t i = 0; i < sizeof simple / sizeof simple[0]; i++) {
        if (!strcmp(op, simple[i][0])) return request(simple[i][1], NULL, NULL, err, n);
    }
    if (!strcmp(op, "scene")) {
        if (!name) {
            snprintf(err, n, "scene: missing name");
            return false;
        }
        return request("SetCurrentProgramScene", one("sceneName", name), NULL, err, n);
    }
    if (!strcmp(op, "mute_toggle")) {
        if (!input) {
            snprintf(err, n, "mute_toggle: missing input");
            return false;
        }
        return request("ToggleInputMute", one("inputName", input), NULL, err, n);
    }
    if (!strcmp(op, "source_visible")) {
        const char *scene = cJSON_GetStringValue(cJSON_GetObjectItem(args, "scene"));
        const char *item = cJSON_GetStringValue(cJSON_GetObjectItem(args, "item"));
        if (!scene || !item) {
            snprintf(err, n, "source_visible: missing scene or item");
            return false;
        }
        cJSON *d = one("sceneName", scene), *r = NULL;
        cJSON_AddStringToObject(d, "sourceName", item);
        if (!request("GetSceneItemId", d, &r, err, n)) return false;
        double id = cJSON_GetNumberValue(cJSON_GetObjectItem(r, "sceneItemId"));
        cJSON_Delete(r);
        // visible: true / false / "toggle" (default)
        const cJSON *vis = cJSON_GetObjectItem(args, "visible");
        bool want;
        if (cJSON_IsBool(vis) || (cJSON_IsString(vis) && strcmp(vis->valuestring, "toggle"))) {
            want = cJSON_IsTrue(vis) || (cJSON_IsString(vis) && !strcmp(vis->valuestring, "true"));
        } else {
            cJSON *g = one("sceneName", scene);
            cJSON_AddNumberToObject(g, "sceneItemId", id);
            r = NULL;
            if (!request("GetSceneItemEnabled", g, &r, err, n)) return false;
            want = !cJSON_IsTrue(cJSON_GetObjectItem(r, "sceneItemEnabled"));
            cJSON_Delete(r);
        }
        cJSON *s = one("sceneName", scene);
        cJSON_AddNumberToObject(s, "sceneItemId", id);
        cJSON_AddBoolToObject(s, "sceneItemEnabled", want);
        return request("SetSceneItemEnabled", s, NULL, err, n);
    }
    snprintf(err, n, "obs: unknown op %s", op);
    return false;
}

const plugin_t plugin_obs = {
    .name = "obs",
    .prefix = "obs",
    .enabled = enabled,
    .start = start,
    .ready = ready,
    .run = run,
};
