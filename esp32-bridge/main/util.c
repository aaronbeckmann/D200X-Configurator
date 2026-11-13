// HTTP client, JSON pointer and Base64 helpers shared by the plugins.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "esp_crt_bundle.h"
#include "esp_heap_caps.h"
#include "esp_http_client.h"
#include "esp_timer.h"

#define HTTP_MAX_BODY (64 * 1024)

int64_t now_ms(void) {
    return esp_timer_get_time() / 1000;
}

typedef struct {
    char *buf;
    size_t len, cap;
} body_t;

static esp_err_t on_http_event(esp_http_client_event_t *e) {
    body_t *b = e->user_data;
    if (e->event_id != HTTP_EVENT_ON_DATA || !b) {
        return ESP_OK;
    }
    if (b->len + e->data_len + 1 > b->cap) {
        size_t cap = b->cap ? b->cap * 2 : 2048;
        while (cap < b->len + e->data_len + 1) {
            cap *= 2;
        }
        if (cap > HTTP_MAX_BODY) {
            return ESP_OK;  // keep what fits
        }
        char *n = heap_caps_realloc(b->buf, cap, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
        if (!n) {
            return ESP_OK;
        }
        b->buf = n;
        b->cap = cap;
    }
    memcpy(b->buf + b->len, e->data, e->data_len);
    b->len += e->data_len;
    b->buf[b->len] = 0;
    return ESP_OK;
}

int http_request(const char *method, const char *url, const cJSON *headers, const char *content_type, const char *body, char **out) {
    body_t b = {0};
    esp_http_client_method_t m = HTTP_METHOD_GET;
    if (!strcasecmp(method, "POST")) m = HTTP_METHOD_POST;
    else if (!strcasecmp(method, "PUT")) m = HTTP_METHOD_PUT;
    else if (!strcasecmp(method, "DELETE")) m = HTTP_METHOD_DELETE;
    else if (!strcasecmp(method, "PATCH")) m = HTTP_METHOD_PATCH;
    else if (!strcasecmp(method, "HEAD")) m = HTTP_METHOD_HEAD;
    esp_http_client_config_t c = {
        .url = url,
        .method = m,
        .timeout_ms = 8000,
        .event_handler = on_http_event,
        .user_data = &b,
        .crt_bundle_attach = esp_crt_bundle_attach,
        .buffer_size = 2048,
        .buffer_size_tx = 2048,
    };
    esp_http_client_handle_t h = esp_http_client_init(&c);
    if (!h) {
        return -1;
    }
    const cJSON *hd;
    cJSON_ArrayForEach(hd, headers) {
        if (cJSON_IsString(hd)) {
            esp_http_client_set_header(h, hd->string, hd->valuestring);
        }
    }
    if (body) {
        esp_http_client_set_header(h, "Content-Type", content_type ? content_type : "application/json");
        esp_http_client_set_post_field(h, body, strlen(body));
    } else if (m == HTTP_METHOD_POST || m == HTTP_METHOD_PUT) {
        // Spotify and others want an explicit empty body
        esp_http_client_set_post_field(h, "", 0);
    }
    int status = -1;
    if (esp_http_client_perform(h) == ESP_OK) {
        status = esp_http_client_get_status_code(h);
    }
    esp_http_client_cleanup(h);
    if (out) {
        *out = b.buf ? b.buf : strdup("");
    } else {
        free(b.buf);
    }
    return status;
}

void json_value_text(const cJSON *v, char *out, size_t n) {
    if (!n) return;
    out[0] = 0;
    if (!v) return;
    if (cJSON_IsString(v)) {
        snprintf(out, n, "%s", v->valuestring);
        return;
    }
    char *s = cJSON_PrintUnformatted(v);
    if (s) {
        snprintf(out, n, "%s", s);
        free(s);
    }
}

void json_pointer_text(const cJSON *root, const char *pointer, char *out, size_t n) {
    const cJSON *cur = root;
    const char *p = pointer ? pointer : "";
    char part[96];
    while (*p == '/' && cur) {
        p++;
        size_t k = 0;
        while (*p && *p != '/' && k + 1 < sizeof part) {
            part[k++] = *p++;
        }
        part[k] = 0;
        if (cJSON_IsArray(cur)) {
            cur = cJSON_GetArrayItem(cur, atoi(part));
        } else {
            cur = cJSON_GetObjectItemCaseSensitive(cur, part);
        }
    }
    json_value_text(cur, out, n);
}

void base64_encode(const uint8_t *in, size_t len, char *out, size_t outlen) {
    static const char t[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    size_t o = 0;
    for (size_t i = 0; i < len && o + 5 < outlen; i += 3) {
        uint32_t v = in[i] << 16 | (i + 1 < len ? in[i + 1] << 8 : 0) | (i + 2 < len ? in[i + 2] : 0);
        out[o++] = t[v >> 18 & 63];
        out[o++] = t[v >> 12 & 63];
        out[o++] = i + 1 < len ? t[v >> 6 & 63] : '=';
        out[o++] = i + 2 < len ? t[v & 63] : '=';
    }
    out[o] = 0;
}
