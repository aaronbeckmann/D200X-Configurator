// Plugins defined in the config, so new integrations need no firmware update.
//
// "plugins": {
//   "wled": {
//     "conn": "wled",                         // base URL + headers from "connections", or:
//     "url": "http://192.168.1.50", "headers": {"X-Key": "..."},
//     "ops": {
//       "toggle": {"method": "POST", "path": "/json/state", "body": {"on": "t"}},
//       "bri": {"press": {"method": "POST", "path": "/json/state", "body": {"on": "t"}},
//               "turn":  {"method": "POST", "path": "/json/state",
//                         "body": {"bri": "${clamp(state:bri + delta * 16, 1, 255)}"}}},
//       "preset": {"type": "mqtt", "topic": "wled/all/api", "payload": "PL=${args.id}"},
//       "both": [ {"path": "/a"}, {"type": "udp", "host": "10.0.0.9", "port": 4210, "payload": "x"} ]
//     },
//     "states": {
//       "on":   {"path": "/json/state", "pointer": "/on", "every_s": 5},
//       "bri":  {"path": "/json/state", "pointer": "/bri", "every_s": 5},
//       "door": {"topic": "home/door", "map": {"1": "open", "0": "closed"}}
//     },
//     "refresh": true                         // poll the states right after an op (default)
//   }
// }
//
// Deck keys: {"type":"host","plugin":"wled","op":"bri"}; tiles: states "wled:on", "wled:bri", ...
//
// Op templates: "type" http (default: method, path | url, headers, body, content_type), mqtt
// (topic, payload, retain, qos), udp (host, port, payload | hex), wol (mac, broadcast, port); an
// array runs its templates in order; an object with "press" / "turn" picks one by the event.
// Strings may contain ${expr}; a string that is only "${expr}" keeps the value's JSON type.
// expr: numbers, + - * / ( ), delta, event, args.<path>, state:<key> (this plugin) or
// state:<prefix>:<name> (any state), min(a,b) max(a,b) clamp(x,lo,hi) round(x) not(x) bool(x).
#include <ctype.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/netdb.h"
#include "lwip/sockets.h"

static const char *TAG = "decl";

typedef struct {
    char name[32];
    const cJSON *def;
    int64_t *due;  // next poll per state (index in "states")
    int nstates;
} decl_t;

static plugin_t *plugins;
static size_t count;

// ---- expressions --------------------------------------------------------------------------------

typedef struct {
    bool num;           // numeric value in n
    bool is_bool;       // bool(...) / not(...) result
    double n;
    char s[256];        // text form
    const cJSON *json;  // the original JSON value (args.x), for type-preserving substitution
} val_t;

typedef struct {
    const char *p;
    const decl_t *d;
    const cJSON *args;
    const char *event;
    int delta;
} ectx_t;

static val_t expr(ectx_t *c);

static void ws(ectx_t *c) {
    while (*c->p == ' ') c->p++;
}

static void set_num(val_t *v, double n) {
    v->num = true;
    v->n = n;
    if (n == floor(n) && fabs(n) < 1e15) snprintf(v->s, sizeof v->s, "%lld", (long long)n);
    else snprintf(v->s, sizeof v->s, "%g", n);
}

static void set_text(val_t *v, const char *s) {
    snprintf(v->s, sizeof v->s, "%s", s);
    char *end;
    v->n = strtod(v->s, &end);
    v->num = end != v->s && *end == 0;
}

static bool truthy(const val_t *v) {
    if (v->num) return v->n != 0;
    return !strcasecmp(v->s, "true") || !strcasecmp(v->s, "on") || !strcasecmp(v->s, "yes") || !strcasecmp(v->s, "open");
}

static val_t ident(ectx_t *c, const char *id) {
    val_t v = {0};
    if (!strcmp(id, "delta")) {
        set_num(&v, c->delta);
    } else if (!strcmp(id, "event")) {
        set_text(&v, c->event);
    } else if (!strncmp(id, "args.", 5)) {
        const cJSON *cur = c->args;
        char path[192];
        snprintf(path, sizeof path, "%s", id + 5);
        for (char *part = strtok(path, "."); part && cur; part = strtok(NULL, ".")) {
            cur = cJSON_IsArray(cur) ? cJSON_GetArrayItem(cur, atoi(part)) : cJSON_GetObjectItemCaseSensitive(cur, part);
        }
        v.json = cur;
        if (cJSON_IsNumber(cur)) set_num(&v, cur->valuedouble);
        else if (cJSON_IsBool(cur)) { set_num(&v, cJSON_IsTrue(cur)); v.is_bool = true; snprintf(v.s, sizeof v.s, cJSON_IsTrue(cur) ? "true" : "false"); }
        else if (cJSON_IsString(cur)) set_text(&v, cur->valuestring);
        else json_value_text(cur, v.s, sizeof v.s);
    } else if (!strncmp(id, "state:", 6)) {
        char name[192], val[256] = "";
        if (strchr(id + 6, ':')) snprintf(name, sizeof name, "%s", id + 6);
        else snprintf(name, sizeof name, "%s:%s", c->d->name, id + 6);
        store_get(name, val, sizeof val);
        set_text(&v, val);
    } else {
        set_text(&v, id);  // unknown word: as text
    }
    return v;
}

static val_t call_fn(ectx_t *c, const char *fn) {
    val_t a[3] = {0};
    int n = 0;
    c->p++;  // '('
    ws(c);
    while (*c->p && *c->p != ')' && n < 3) {
        a[n++] = expr(c);
        ws(c);
        if (*c->p == ',') c->p++;
        ws(c);
    }
    if (*c->p == ')') c->p++;
    val_t r = {0};
    if (!strcmp(fn, "min") && n == 2) set_num(&r, fmin(a[0].n, a[1].n));
    else if (!strcmp(fn, "max") && n == 2) set_num(&r, fmax(a[0].n, a[1].n));
    else if (!strcmp(fn, "clamp") && n == 3) set_num(&r, fmin(fmax(a[0].n, a[1].n), a[2].n));
    else if (!strcmp(fn, "round") && n == 1) set_num(&r, round(a[0].n));
    else if ((!strcmp(fn, "not") || !strcmp(fn, "bool")) && n == 1) {
        bool b = truthy(&a[0]) ^ !strcmp(fn, "not");
        set_num(&r, b);
        r.is_bool = true;
        snprintf(r.s, sizeof r.s, b ? "true" : "false");
    } else ESP_LOGW(TAG, "unknown function %s/%d", fn, n);
    return r;
}

static val_t primary(ectx_t *c) {
    ws(c);
    val_t v = {0};
    if (*c->p == '(') {
        c->p++;
        v = expr(c);
        ws(c);
        if (*c->p == ')') c->p++;
    } else if (*c->p == '-') {
        c->p++;
        val_t x = primary(c);
        set_num(&v, -x.n);
    } else if (isdigit((unsigned char)*c->p) || *c->p == '.') {
        char *end;
        double n = strtod(c->p, &end);
        c->p = end;
        set_num(&v, n);
    } else {
        char id[160];
        size_t k = 0;
        while ((isalnum((unsigned char)*c->p) || strchr("_.:", *c->p)) && *c->p && k + 1 < sizeof id) id[k++] = *c->p++;
        id[k] = 0;
        ws(c);
        if (*c->p == '(') return call_fn(c, id);
        v = ident(c, id);
    }
    return v;
}

static val_t term(ectx_t *c) {
    val_t v = primary(c);
    for (;;) {
        ws(c);
        char op = *c->p;
        if (op != '*' && op != '/') return v;
        c->p++;
        val_t r = primary(c);
        double n = op == '*' ? v.n * r.n : (r.n != 0 ? v.n / r.n : 0);
        v = (val_t){0};
        set_num(&v, n);
    }
}

static val_t expr(ectx_t *c) {
    val_t v = term(c);
    for (;;) {
        ws(c);
        char op = *c->p;
        if (op != '+' && op != '-') return v;
        c->p++;
        val_t r = term(c);
        double n = op == '+' ? v.n + r.n : v.n - r.n;
        v = (val_t){0};
        set_num(&v, n);
    }
}

static val_t eval(const decl_t *d, const char *src, const cJSON *args, const char *event, int delta) {
    ectx_t c = {.p = src, .d = d, .args = args, .event = event, .delta = delta};
    return expr(&c);
}

// Copy of a template with every ${...} filled in.
static cJSON *fill(const decl_t *d, const cJSON *t, const cJSON *args, const char *event, int delta) {
    if (cJSON_IsString(t)) {
        const char *s = t->valuestring;
        size_t len = strlen(s);
        bool whole = len > 3 && !strncmp(s, "${", 2) && strchr(s + 2, '}') == s + len - 1;
        if (whole) {
            char e[256];
            snprintf(e, sizeof e, "%.*s", (int)(len - 3), s + 2);
            val_t v = eval(d, e, args, event, delta);
            if (v.json && !cJSON_IsString(v.json)) return cJSON_Duplicate(v.json, true);
            if (v.is_bool) return cJSON_CreateBool(v.n != 0);
            if (v.num && !v.json) return cJSON_CreateNumber(v.n);
            return cJSON_CreateString(v.s);
        }
        char out[1024];
        size_t o = 0;
        for (const char *p = s; *p && o + 1 < sizeof out;) {
            const char *close = !strncmp(p, "${", 2) ? strchr(p, '}') : NULL;
            if (close) {
                char e[256];
                snprintf(e, sizeof e, "%.*s", (int)(close - p - 2), p + 2);
                val_t v = eval(d, e, args, event, delta);
                o += snprintf(out + o, sizeof out - o, "%s", v.s);
                if (o >= sizeof out) o = sizeof out - 1;
                p = close + 1;
            } else {
                out[o++] = *p++;
            }
        }
        out[o] = 0;
        return cJSON_CreateString(out);
    }
    if (cJSON_IsObject(t) || cJSON_IsArray(t)) {
        cJSON *r = cJSON_IsObject(t) ? cJSON_CreateObject() : cJSON_CreateArray();
        const cJSON *x;
        cJSON_ArrayForEach(x, t) {
            cJSON *f = fill(d, x, args, event, delta);
            if (cJSON_IsObject(t)) cJSON_AddItemToObject(r, x->string, f);
            else cJSON_AddItemToArray(r, f);
        }
        return r;
    }
    return cJSON_Duplicate(t, true);
}

// ---- requests ----------------------------------------------------------------------------------

// Base URL and headers of the plugin (own "url"/"headers" or a named connection).
static void base_of(const decl_t *d, char *url, size_t n, cJSON *headers) {
    const cJSON *src = d->def;
    const char *conn = cfg_str(d->def, "conn", NULL);
    if (conn) src = cJSON_GetObjectItem(cfg_section("connections"), conn);
    snprintf(url, n, "%s", cfg_str(src, "url", ""));
    const cJSON *h;
    cJSON_ArrayForEach(h, cJSON_GetObjectItem(src, "headers")) cJSON_AddItemToObject(headers, h->string, cJSON_Duplicate(h, true));
}

// HTTP from a filled template; the response body in *out if wanted.
static int http_of(const decl_t *d, const cJSON *t, char **out) {
    char url[384];
    cJSON *headers = cJSON_CreateObject();
    base_of(d, url, sizeof url, headers);
    const char *full = cfg_str(t, "url", NULL);
    if (full) snprintf(url, sizeof url, "%s", full);
    else strncat(url, cfg_str(t, "path", ""), sizeof url - strlen(url) - 1);
    const cJSON *h;
    cJSON_ArrayForEach(h, cJSON_GetObjectItem(t, "headers")) {
        cJSON_DeleteItemFromObjectCaseSensitive(headers, h->string);
        cJSON_AddItemToObject(headers, h->string, cJSON_Duplicate(h, true));
    }
    const cJSON *b = cJSON_GetObjectItem(t, "body");
    char *body = b ? (cJSON_IsString(b) ? strdup(b->valuestring) : cJSON_PrintUnformatted(b)) : NULL;
    int st = http_request(cfg_str(t, "method", body ? "POST" : "GET"), url, headers, cfg_str(t, "content_type", NULL), body, out);
    free(body);
    cJSON_Delete(headers);
    return st;
}

static bool udp_send(const char *host, int port, const uint8_t *data, size_t len, bool bcast) {
    struct addrinfo hints = {.ai_family = AF_INET, .ai_socktype = SOCK_DGRAM}, *res = NULL;
    char ps[8];
    snprintf(ps, sizeof ps, "%d", port);
    if (getaddrinfo(host, ps, &hints, &res) || !res) return false;
    int s = socket(AF_INET, SOCK_DGRAM, 0), on = 1;
    if (bcast) setsockopt(s, SOL_SOCKET, SO_BROADCAST, &on, sizeof on);
    int r = sendto(s, data, len, 0, res->ai_addr, res->ai_addrlen);
    close(s);
    freeaddrinfo(res);
    return r >= 0;
}

// One filled template.
static bool exec_one(const decl_t *d, const cJSON *t, char *err, size_t n) {
    const char *type = cfg_str(t, "type", "http");
    if (!strcmp(type, "http")) {
        int st = http_of(d, t, NULL);
        if (st >= 200 && st < 300) return true;
        snprintf(err, n, st < 0 ? "%s: no connection" : "%s: HTTP %d", d->name, st);
        return false;
    }
    if (!strcmp(type, "mqtt")) {
        char payload[512];
        json_value_text(cJSON_GetObjectItem(t, "payload"), payload, sizeof payload);
        if (mqtt_publish(cfg_str(t, "topic", ""), payload, (int)cJSON_GetNumberValue(cJSON_GetObjectItem(t, "qos")), cJSON_IsTrue(cJSON_GetObjectItem(t, "retain")))) return true;
        snprintf(err, n, "%s: mqtt not connected", d->name);
        return false;
    }
    if (!strcmp(type, "udp") || !strcmp(type, "wol")) {
        uint8_t buf[512];
        size_t len = 0;
        bool wol = !strcmp(type, "wol");
        if (wol) {
            unsigned m[6];
            if (sscanf(cfg_str(t, "mac", ""), "%x%*[:-]%x%*[:-]%x%*[:-]%x%*[:-]%x%*[:-]%x", &m[0], &m[1], &m[2], &m[3], &m[4], &m[5]) != 6) {
                snprintf(err, n, "%s: bad mac", d->name);
                return false;
            }
            memset(buf, 0xff, 6);
            for (int i = 0; i < 16; i++)
                for (int k = 0; k < 6; k++) buf[6 + i * 6 + k] = m[k];
            len = 102;
        } else if (cfg_str(t, "hex", NULL)) {
            for (const char *p = cfg_str(t, "hex", ""); p[0] && p[1] && len < sizeof buf;) {
                unsigned v;
                if (*p == ' ' || *p == ':') { p++; continue; }
                if (sscanf(p, "%2x", &v) != 1) break;
                buf[len++] = v;
                p += 2;
            }
        } else {
            char pl[512];
            json_value_text(cJSON_GetObjectItem(t, "payload"), pl, sizeof pl);
            len = strlen(pl);
            memcpy(buf, pl, len);
        }
        const cJSON *port = cJSON_GetObjectItem(t, "port");
        const char *host = cfg_str(t, wol ? "broadcast" : "host", wol ? "255.255.255.255" : "");
        if (udp_send(host, cJSON_IsNumber(port) ? port->valueint : 9, buf, len, wol)) return true;
        snprintf(err, n, "%s: udp to %s failed", d->name, host);
        return false;
    }
    snprintf(err, n, "%s: unknown type %s", d->name, type);
    return false;
}

// ---- states --------------------------------------------------------------------------------------

static void set_state(const decl_t *d, const cJSON *def, const char *key, const char *raw) {
    char name[96], val[256];
    snprintf(name, sizeof name, "%s:%s", d->name, key);
    snprintf(val, sizeof val, "%s", raw);
    const cJSON *mapped = cJSON_GetObjectItemCaseSensitive(cJSON_GetObjectItem(def, "map"), raw);
    if (cJSON_IsString(mapped)) snprintf(val, sizeof val, "%s", mapped->valuestring);
    store_set(name, val);
}

static void poll_state(const decl_t *d, const cJSON *s) {
    char *resp = NULL;
    int st = http_of(d, s, &resp);
    if (st >= 200 && st < 300) {
        char val[256];
        const char *ptr = cfg_str(s, "pointer", NULL);
        if (ptr) {
            cJSON *j = cJSON_Parse(resp);
            json_pointer_text(j, ptr, val, sizeof val);
            cJSON_Delete(j);
        } else {
            snprintf(val, sizeof val, "%s", resp ? resp : "");
            val[strcspn(val, "\r\n")] = 0;
        }
        set_state(d, s, s->string, val);
    }
    free(resp);
}

static void poll_task(void *arg) {
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(250));
        if (!wifi_up()) continue;
        int64_t t = now_ms();
        for (size_t i = 0; i < count; i++) {
            decl_t *d = plugins[i].ctx;
            int k = 0;
            const cJSON *s;
            cJSON_ArrayForEach(s, cJSON_GetObjectItem(d->def, "states")) {
                if (k < d->nstates && !cJSON_GetObjectItem(s, "topic") && t >= d->due[k]) {
                    const cJSON *every = cJSON_GetObjectItem(s, "every_s");
                    d->due[k] = t + (int64_t)(cJSON_IsNumber(every) && every->valuedouble > 0 ? every->valuedouble : 30) * 1000;
                    poll_state(d, s);
                }
                k++;
            }
        }
    }
}

void decl_mqtt_subscribe(void (*sub)(const char *topic)) {
    for (size_t i = 0; i < count; i++) {
        const decl_t *d = plugins[i].ctx;
        const cJSON *s;
        cJSON_ArrayForEach(s, cJSON_GetObjectItem(d->def, "states")) {
            const char *topic = cfg_str(s, "topic", NULL);
            if (topic) sub(topic);
        }
    }
}

void decl_mqtt_message(const char *topic, int topic_len, const char *payload) {
    for (size_t i = 0; i < count; i++) {
        const decl_t *d = plugins[i].ctx;
        const cJSON *s;
        cJSON_ArrayForEach(s, cJSON_GetObjectItem(d->def, "states")) {
            const char *filter = cfg_str(s, "topic", NULL);
            if (!filter || !mqtt_topic_matches(filter, topic, topic_len)) continue;
            const char *ptr = cfg_str(s, "pointer", NULL);
            char val[256];
            if (ptr) {
                cJSON *j = cJSON_Parse(payload);
                json_pointer_text(j, ptr, val, sizeof val);
                cJSON_Delete(j);
            } else {
                snprintf(val, sizeof val, "%s", payload);
            }
            set_state(d, s, s->string, val);
        }
    }
}

// ---- plugin interface ----------------------------------------------------------------------------

static bool enabled(const plugin_t *self) {
    return true;
}

static bool ready(const plugin_t *self) {
    return wifi_up();
}

static void start(const plugin_t *self) {
    static bool poller;
    if (!poller) {
        poller = true;
        xTaskCreate(poll_task, "decl_poll", 6144, NULL, 3, NULL);
    }
}

static bool run(const plugin_t *self, const char *op, const cJSON *args, const char *event, int delta, char *err, size_t n) {
    const decl_t *d = self->ctx;
    const cJSON *t = cJSON_GetObjectItemCaseSensitive(cJSON_GetObjectItem(d->def, "ops"), op);
    if (!t) {
        snprintf(err, n, "%s: unknown op %s", d->name, op);
        return false;
    }
    if (cJSON_IsObject(t) && (cJSON_GetObjectItem(t, "press") || cJSON_GetObjectItem(t, "turn"))) {
        t = cJSON_GetObjectItem(t, event);
        if (!t) return true;  // nothing for this event
    }
    cJSON *filled = fill(d, t, args, event, delta);
    bool ok = true;
    if (cJSON_IsArray(filled)) {
        const cJSON *x;
        cJSON_ArrayForEach(x, filled) if (!(ok = exec_one(d, x, err, n))) break;
    } else {
        ok = exec_one(d, filled, err, n);
    }
    cJSON_Delete(filled);
    const cJSON *refresh = cJSON_GetObjectItem(d->def, "refresh");
    if (ok && !cJSON_IsFalse(refresh)) {
        for (int k = 0; k < d->nstates; k++) d->due[k] = 0;  // poll right away
    }
    return ok;
}

static bool reserved(const char *name) {
    static const char *taken[] = {"homeassistant", "ha", "mqtt", "net", "obs", "spotify", "system", "media", "discord", "agentdeck", "hid"};
    for (size_t i = 0; i < sizeof taken / sizeof taken[0]; i++)
        if (!strcmp(name, taken[i])) return true;
    for (const char *p = name; *p; p++)
        if (!islower((unsigned char)*p) && !isdigit((unsigned char)*p) && *p != '_' && *p != '-') return true;
    return !*name;
}

plugin_t *decl_plugins_create(size_t *out) {
    const cJSON *defs = cfg_section("plugins");
    int max = cJSON_GetArraySize(defs);
    plugins = calloc(max ? max : 1, sizeof *plugins);
    count = 0;
    const cJSON *def;
    cJSON_ArrayForEach(def, defs) {
        if (!cJSON_IsObject(def) || reserved(def->string) || strlen(def->string) >= 32) {
            ESP_LOGW(TAG, "plugin name '%s' skipped (reserved or not lowercase a-z 0-9 _ -)", def->string);
            continue;
        }
        decl_t *d = calloc(1, sizeof *d);
        snprintf(d->name, sizeof d->name, "%s", def->string);
        d->def = def;
        d->nstates = cJSON_GetArraySize(cJSON_GetObjectItem(def, "states"));
        d->due = calloc(d->nstates ? d->nstates : 1, sizeof *d->due);
        plugins[count] = (plugin_t){
            .name = d->name,
            .prefix = d->name,
            .enabled = enabled,
            .start = start,
            .ready = ready,
            .run = run,
            .ctx = d,
        };
        count++;
    }
    *out = count;
    return plugins;
}
