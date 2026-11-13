// Spotify Web API with the refresh token from the configurator's PKCE login. Same ops and states
// as the PC agent (d200x-agent/src/spotify.rs), without album art.
//   config "spotify": {"client_id": "...", "refresh_token": "..."}
//   states spotify:connected, playing, title, artist, album, line, shuffle, repeat, liked, volume, device
// Spotify rotates refresh tokens; the new one is written back to the config.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"

#define API "https://api.spotify.com/v1"

static SemaphoreHandle_t lock;
static char access[400];
static int64_t access_until;
static volatile bool connected;
static volatile bool auth_failed;
static char track_id[40];

static bool enabled(const plugin_t *self) {
    const cJSON *s = cfg_section("spotify");
    return *cfg_str(s, "client_id", "") && *cfg_str(s, "refresh_token", "");
}

// Ready while Wi-Fi is up and the refresh token works; a key press fetches a token if needed.
static bool ready(const plugin_t *self) {
    return wifi_up() && !auth_failed;
}

static bool refresh_token(void) {
    const cJSON *s = cfg_section("spotify");
    char *body = malloc(1024);
    snprintf(body, 1024, "grant_type=refresh_token&refresh_token=%s&client_id=%s", cfg_str(s, "refresh_token", ""), cfg_str(s, "client_id", ""));
    char *resp = NULL;
    int st = http_request("POST", "https://accounts.spotify.com/api/token", NULL, "application/x-www-form-urlencoded", body, &resp);
    free(body);
    cJSON *j = cJSON_Parse(resp);
    free(resp);
    const char *tok = cJSON_GetStringValue(cJSON_GetObjectItem(j, "access_token"));
    bool ok = st == 200 && tok;
    // a rejected token (400/401) needs a new login; a network failure (-1) may pass
    auth_failed = st == 400 || st == 401;
    if (ok) {
        snprintf(access, sizeof access, "%s", tok);
        access_until = now_ms() + ((int64_t)cJSON_GetNumberValue(cJSON_GetObjectItem(j, "expires_in")) - 60) * 1000;
        const char *rot = cJSON_GetStringValue(cJSON_GetObjectItem(j, "refresh_token"));
        if (rot && strcmp(rot, cfg_str(s, "refresh_token", ""))) cfg_update_string("spotify", "refresh_token", rot);
    } else {
        link_log("warn", "spotify: token refresh failed (HTTP %d)", st);
    }
    cJSON_Delete(j);
    return ok;
}

// API call; refreshes the token when needed and once more on 401. Returns the HTTP status.
static int api(const char *method, const char *path, const char *body, char **out) {
    xSemaphoreTake(lock, portMAX_DELAY);
    int st = -1;
    for (int attempt = 0; attempt < 2; attempt++) {
        if ((!access[0] || now_ms() > access_until || attempt) && !refresh_token()) break;
        char url[300], auth[420];
        snprintf(url, sizeof url, API "%s", path);
        snprintf(auth, sizeof auth, "Bearer %s", access);
        cJSON *h = cJSON_CreateObject();
        cJSON_AddStringToObject(h, "Authorization", auth);
        if (out) *out = NULL;
        st = http_request(method, url, h, NULL, body, out);
        cJSON_Delete(h);
        if (st != 401) break;
        if (out) {
            free(*out);
            *out = NULL;
        }
    }
    xSemaphoreGive(lock);
    connected = st >= 200 && st < 500 && st != 401;
    store_set("spotify:connected", connected ? "true" : "false");
    return st;
}

static void bool_state(const char *name, const cJSON *v) {
    store_set(name, cJSON_IsTrue(v) ? "true" : "false");
}

static void poll_player(void) {
    char *resp = NULL;
    int st = api("GET", "/me/player", NULL, &resp);
    if (st == 204) {
        store_set("spotify:playing", "false");
    } else if (st == 200) {
        cJSON *v = cJSON_Parse(resp);
        bool_state("spotify:playing", cJSON_GetObjectItem(v, "is_playing"));
        bool_state("spotify:shuffle", cJSON_GetObjectItem(v, "shuffle_state"));
        store_set("spotify:repeat", cJSON_GetStringValue(cJSON_GetObjectItem(v, "repeat_state")) ?: "off");
        const cJSON *dev = cJSON_GetObjectItem(v, "device");
        char num[16];
        snprintf(num, sizeof num, "%d", (int)cJSON_GetNumberValue(cJSON_GetObjectItem(dev, "volume_percent")));
        store_set("spotify:volume", num);
        store_set("spotify:device", cJSON_GetStringValue(cJSON_GetObjectItem(dev, "name")) ?: "");
        const cJSON *item = cJSON_GetObjectItem(v, "item");
        const char *id = cJSON_GetStringValue(cJSON_GetObjectItem(item, "id"));
        if (item && id && strcmp(id, track_id)) {
            snprintf(track_id, sizeof track_id, "%s", id);
            const char *title = cJSON_GetStringValue(cJSON_GetObjectItem(item, "name")) ?: "";
            char artist[160] = "";
            const cJSON *a;
            cJSON_ArrayForEach(a, cJSON_GetObjectItem(item, "artists")) {
                size_t l = strlen(artist);
                snprintf(artist + l, sizeof artist - l, "%s%s", l ? ", " : "", cJSON_GetStringValue(cJSON_GetObjectItem(a, "name")) ?: "");
            }
            store_set("spotify:title", title);
            store_set("spotify:artist", artist);
            store_set("spotify:album", cJSON_GetStringValue(cJSON_GetObjectItem(cJSON_GetObjectItem(item, "album"), "name")) ?: "");
            char line[256];
            snprintf(line, sizeof line, *artist ? "%s - %s" : "%s%s", artist, title);
            store_set("spotify:line", line);
            char path[80], *lr = NULL;
            snprintf(path, sizeof path, "/me/tracks/contains?ids=%s", id);
            if (api("GET", path, NULL, &lr) == 200) {
                cJSON *l = cJSON_Parse(lr);
                bool_state("spotify:liked", cJSON_GetArrayItem(l, 0));
                cJSON_Delete(l);
            }
            free(lr);
        }
        cJSON_Delete(v);
    }
    free(resp);
}

static void spotify_task(void *arg) {
    for (;;) {
        // poll quickly only while the deck shows Spotify states
        cJSON *w = store_wanted_with_prefix("spotify:");
        bool shown = cJSON_GetArraySize(w) > 0;
        cJSON_Delete(w);
        if (wifi_up() && shown) poll_player();
        vTaskDelay(pdMS_TO_TICKS(shown ? 3000 : 10000));
    }
}

static void start(const plugin_t *self) {
    lock = xSemaphoreCreateMutex();
    store_set("spotify:connected", "false");
    xTaskCreate(spotify_task, "spotify", 8192, NULL, 3, NULL);
}

static bool is_true(const char *state) {
    char v[16] = "";
    store_get(state, v, sizeof v);
    return !strcmp(v, "true");
}

static bool call(const char *method, const char *path, const char *body, char *err, size_t n) {
    int st = api(method, path, body, NULL);
    if (st >= 200 && st < 300) return true;
    snprintf(err, n, st == 404 ? "no active Spotify device" : "%s %s: HTTP %d", method, path, st);
    return false;
}

static bool run(const plugin_t *self, const char *op, const cJSON *args, const char *event, int delta, char *err, size_t n) {
    char path[160];
    if (!strcmp(op, "play_pause")) return call("PUT", is_true("spotify:playing") ? "/me/player/pause" : "/me/player/play", NULL, err, n);
    if (!strcmp(op, "play")) {
        const char *uri = cJSON_GetStringValue(cJSON_GetObjectItem(args, "uri"));
        if (!uri) return call("PUT", "/me/player/play", NULL, err, n);
        cJSON *b = cJSON_CreateObject();
        if (strstr(uri, ":track:")) {
            cJSON *a = cJSON_AddArrayToObject(b, "uris");
            cJSON_AddItemToArray(a, cJSON_CreateString(uri));
        } else {
            cJSON_AddStringToObject(b, "context_uri", uri);
        }
        char *body = cJSON_PrintUnformatted(b);
        cJSON_Delete(b);
        bool ok = call("PUT", "/me/player/play", body, err, n);
        free(body);
        return ok;
    }
    if (!strcmp(op, "next")) return call("POST", "/me/player/next", NULL, err, n);
    if (!strcmp(op, "prev")) return call("POST", "/me/player/previous", NULL, err, n);
    if (!strcmp(op, "shuffle_toggle")) {
        snprintf(path, sizeof path, "/me/player/shuffle?state=%s", is_true("spotify:shuffle") ? "false" : "true");
        return call("PUT", path, NULL, err, n);
    }
    if (!strcmp(op, "repeat_cycle")) {
        char cur[16] = "off";
        store_get("spotify:repeat", cur, sizeof cur);
        snprintf(path, sizeof path, "/me/player/repeat?state=%s", !strcmp(cur, "off") ? "context" : !strcmp(cur, "context") ? "track" : "off");
        return call("PUT", path, NULL, err, n);
    }
    if (!strcmp(op, "like_toggle")) {
        if (!track_id[0]) {
            snprintf(err, n, "no current track");
            return false;
        }
        bool liked = is_true("spotify:liked");
        snprintf(path, sizeof path, "/me/tracks?ids=%s", track_id);
        if (!call(liked ? "DELETE" : "PUT", path, NULL, err, n)) return false;
        store_set("spotify:liked", liked ? "false" : "true");
        return true;
    }
    if (!strcmp(op, "volume") || !strcmp(op, "volume_set")) {
        int v;
        if (!strcmp(op, "volume")) {
            char cur[8] = "50";
            store_get("spotify:volume", cur, sizeof cur);
            const cJSON *step = cJSON_GetObjectItem(args, "step");
            v = atoi(cur) + (cJSON_IsNumber(step) ? step->valueint : 5) * delta;
        } else {
            const cJSON *p = cJSON_GetObjectItem(args, "percent");
            v = cJSON_IsNumber(p) ? p->valueint : 50;
        }
        v = v < 0 ? 0 : v > 100 ? 100 : v;
        snprintf(path, sizeof path, "/me/player/volume?volume_percent=%d", v);
        if (!call("PUT", path, NULL, err, n)) return false;
        char num[8];
        snprintf(num, sizeof num, "%d", v);
        store_set("spotify:volume", num);
        return true;
    }
    snprintf(err, n, "spotify: unknown op %s", op);
    return false;
}

const plugin_t plugin_spotify = {
    .name = "spotify",
    .prefix = "spotify",
    .enabled = enabled,
    .start = start,
    .ready = ready,
    .run = run,
};
