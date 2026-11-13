// Generic network actions; needs only Wi-Fi.
//   op "http" {method, url | conn + path, headers, body, state, pointer}
//        conn names an entry in config "connections" ({"url": base, "headers": {...}}), so tokens
//        stay on the bridge. With "state", the response (or the value at the JSON pointer) becomes
//        net:<state>.
//   op "wol"  {mac, broadcast (default 255.255.255.255), port (default 9)}
//   op "udp"  {host, port, payload | hex}
//   config "net": {"polls": [{"state", "url" | "conn"+"path", "every_s", "pointer", "method",
//                             "headers", "body"}]}  -> net:<state>, refreshed every every_s
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/netdb.h"
#include "lwip/sockets.h"

static bool enabled(const plugin_t *self) {
    return true;
}

static bool ready(const plugin_t *self) {
    return wifi_up();
}

// Resolve url / conn+path and merge the connection's headers with the action's.
static bool target(const cJSON *a, char *url, size_t n, cJSON **headers, char *err, size_t errn) {
    *headers = cJSON_CreateObject();
    const char *conn = cJSON_GetStringValue(cJSON_GetObjectItem(a, "conn"));
    const char *u = cJSON_GetStringValue(cJSON_GetObjectItem(a, "url"));
    if (conn) {
        const cJSON *c = cJSON_GetObjectItem(cfg_section("connections"), conn);
        if (!c) {
            snprintf(err, errn, "unknown connection %s", conn);
            return false;
        }
        snprintf(url, n, "%s%s", cfg_str(c, "url", ""), cJSON_GetStringValue(cJSON_GetObjectItem(a, "path")) ?: "");
        const cJSON *h;
        cJSON_ArrayForEach(h, cJSON_GetObjectItem(c, "headers")) cJSON_AddItemToObject(*headers, h->string, cJSON_Duplicate(h, true));
    } else if (u) {
        snprintf(url, n, "%s", u);
    } else {
        snprintf(err, errn, "missing url");
        return false;
    }
    const cJSON *h;
    cJSON_ArrayForEach(h, cJSON_GetObjectItem(a, "headers")) {
        cJSON_DeleteItemFromObjectCaseSensitive(*headers, h->string);
        cJSON_AddItemToObject(*headers, h->string, cJSON_Duplicate(h, true));
    }
    return true;
}

// One request as described by an action or a poll; sets net:<state> if asked.
static bool do_http(const cJSON *a, char *err, size_t n) {
    char url[384];
    cJSON *headers;
    if (!target(a, url, sizeof url, &headers, err, n)) {
        cJSON_Delete(headers);
        return false;
    }
    const cJSON *b = cJSON_GetObjectItem(a, "body");
    char *body = b ? (cJSON_IsString(b) ? strdup(b->valuestring) : cJSON_PrintUnformatted(b)) : NULL;
    const char *method = cJSON_GetStringValue(cJSON_GetObjectItem(a, "method")) ?: (body ? "POST" : "GET");
    char *resp = NULL;
    int st = http_request(method, url, headers, cJSON_GetStringValue(cJSON_GetObjectItem(a, "content_type")), body, &resp);
    cJSON_Delete(headers);
    free(body);
    const char *state = cJSON_GetStringValue(cJSON_GetObjectItem(a, "state"));
    if (state && st >= 200 && st < 300) {
        char name[96], val[256];
        snprintf(name, sizeof name, "net:%s", state);
        const char *ptr = cJSON_GetStringValue(cJSON_GetObjectItem(a, "pointer"));
        if (ptr) {
            cJSON *j = cJSON_Parse(resp);
            json_pointer_text(j, ptr, val, sizeof val);
            cJSON_Delete(j);
        } else {
            snprintf(val, sizeof val, "%s", resp ? resp : "");
            val[strcspn(val, "\r\n")] = 0;
        }
        store_set(name, val);
    }
    free(resp);
    if (st < 200 || st >= 300) {
        snprintf(err, n, st < 0 ? "%s: no connection" : "%s: HTTP %d", url, st);
        return false;
    }
    return true;
}

static bool send_udp(const char *host, int port, const uint8_t *data, size_t len, bool broadcast, char *err, size_t n) {
    struct addrinfo hints = {.ai_family = AF_INET, .ai_socktype = SOCK_DGRAM}, *res = NULL;
    char ps[8];
    snprintf(ps, sizeof ps, "%d", port);
    if (getaddrinfo(host, ps, &hints, &res) || !res) {
        snprintf(err, n, "cannot resolve %s", host);
        return false;
    }
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    int on = 1;
    if (broadcast) setsockopt(s, SOL_SOCKET, SO_BROADCAST, &on, sizeof on);
    int r = sendto(s, data, len, 0, res->ai_addr, res->ai_addrlen);
    close(s);
    freeaddrinfo(res);
    if (r < 0) {
        snprintf(err, n, "send to %s:%d failed", host, port);
        return false;
    }
    return true;
}

static bool run(const plugin_t *self, const char *op, const cJSON *args, const char *event, int delta, char *err, size_t n) {
    if (!strcmp(op, "http")) return do_http(args, err, n);
    if (!strcmp(op, "wol")) {
        const char *mac = cJSON_GetStringValue(cJSON_GetObjectItem(args, "mac")) ?: "";
        unsigned m[6];
        if (sscanf(mac, "%x%*[:-]%x%*[:-]%x%*[:-]%x%*[:-]%x%*[:-]%x", &m[0], &m[1], &m[2], &m[3], &m[4], &m[5]) != 6) {
            snprintf(err, n, "wol: bad mac %s", mac);
            return false;
        }
        uint8_t pkt[102];
        memset(pkt, 0xff, 6);
        for (int i = 0; i < 16; i++)
            for (int k = 0; k < 6; k++) pkt[6 + i * 6 + k] = m[k];
        const cJSON *port = cJSON_GetObjectItem(args, "port");
        return send_udp(cJSON_GetStringValue(cJSON_GetObjectItem(args, "broadcast")) ?: "255.255.255.255", cJSON_IsNumber(port) ? port->valueint : 9, pkt, sizeof pkt, true, err, n);
    }
    if (!strcmp(op, "udp")) {
        const char *host = cJSON_GetStringValue(cJSON_GetObjectItem(args, "host"));
        const cJSON *port = cJSON_GetObjectItem(args, "port");
        if (!host || !cJSON_IsNumber(port)) {
            snprintf(err, n, "udp: missing host or port");
            return false;
        }
        uint8_t buf[512];
        size_t len = 0;
        const char *hex = cJSON_GetStringValue(cJSON_GetObjectItem(args, "hex"));
        if (hex) {
            for (const char *p = hex; p[0] && p[1] && len < sizeof buf;) {
                if (*p == ' ' || *p == ':') { p++; continue; }
                unsigned v;
                if (sscanf(p, "%2x", &v) != 1) break;
                buf[len++] = v;
                p += 2;
            }
        } else {
            const char *pl = cJSON_GetStringValue(cJSON_GetObjectItem(args, "payload")) ?: "";
            len = strlen(pl) < sizeof buf ? strlen(pl) : sizeof buf;
            memcpy(buf, pl, len);
        }
        return send_udp(host, port->valueint, buf, len, false, err, n);
    }
    snprintf(err, n, "net: unknown op %s", op);
    return false;
}

static void poll_task(void *arg) {
    const cJSON *polls = cJSON_GetObjectItem(cfg_section("net"), "polls");
    int count = cJSON_GetArraySize(polls);
    int64_t *due = calloc(count ? count : 1, sizeof *due);
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(500));
        if (!wifi_up()) continue;
        int64_t t = now_ms();
        for (int i = 0; i < count; i++) {
            const cJSON *p = cJSON_GetArrayItem(polls, i);
            if (t < due[i]) continue;
            const cJSON *every = cJSON_GetObjectItem(p, "every_s");
            due[i] = t + (int64_t)(cJSON_IsNumber(every) && every->valueint > 0 ? every->valueint : 60) * 1000;
            char err[160];
            if (!do_http(p, err, sizeof err)) link_log("warn", "net poll %s: %s", cJSON_GetStringValue(cJSON_GetObjectItem(p, "state")) ?: "?", err);
        }
    }
}

static void start(const plugin_t *self) {
    if (cJSON_GetArraySize(cJSON_GetObjectItem(cfg_section("net"), "polls")) > 0) {
        xTaskCreate(poll_task, "net_poll", 6144, NULL, 3, NULL);
    }
}

const plugin_t plugin_net = {
    .name = "net",
    .prefix = "net",
    .enabled = enabled,
    .start = start,
    .ready = ready,
    .run = run,
};
