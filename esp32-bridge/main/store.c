// State store: name -> value, plus the deck's wanted list. Plugins write; the link task flushes
// changed wanted states to the deck (0x0211).
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"

typedef struct {
    char *name;
    char *value;
    bool dirty;
} entry_t;

static SemaphoreHandle_t lock;
static entry_t *entries;
static size_t count, cap;
static char **wanted;
static size_t wanted_count;
static uint32_t wanted_ver;

void store_init(void) {
    lock = xSemaphoreCreateMutex();
}

static entry_t *find(const char *name) {
    for (size_t i = 0; i < count; i++) {
        if (!strcmp(entries[i].name, name)) {
            return &entries[i];
        }
    }
    return NULL;
}

static bool is_wanted(const char *name) {
    for (size_t i = 0; i < wanted_count; i++) {
        if (!strcmp(wanted[i], name)) {
            return true;
        }
    }
    return false;
}

void store_set(const char *name, const char *value) {
    xSemaphoreTake(lock, portMAX_DELAY);
    entry_t *e = find(name);
    if (!e) {
        if (count == cap) {
            cap = cap ? cap * 2 : 64;
            entries = realloc(entries, cap * sizeof *entries);
        }
        e = &entries[count++];
        e->name = strdup(name);
        e->value = strdup(value);
        e->dirty = true;
    } else if (strcmp(e->value, value)) {
        free(e->value);
        e->value = strdup(value);
        e->dirty = true;
    }
    xSemaphoreGive(lock);
}

bool store_get(const char *name, char *out, size_t n) {
    xSemaphoreTake(lock, portMAX_DELAY);
    entry_t *e = find(name);
    if (e && n) {
        strncpy(out, e->value, n - 1);
        out[n - 1] = 0;
    }
    xSemaphoreGive(lock);
    return e != NULL;
}

void store_set_wanted(const cJSON *states) {
    xSemaphoreTake(lock, portMAX_DELAY);
    for (size_t i = 0; i < wanted_count; i++) {
        free(wanted[i]);
    }
    free(wanted);
    wanted_count = 0;
    int n = cJSON_GetArraySize(states);
    wanted = calloc(n ? n : 1, sizeof *wanted);
    const cJSON *s;
    cJSON_ArrayForEach(s, states) {
        if (cJSON_IsString(s)) {
            wanted[wanted_count++] = strdup(s->valuestring);
        }
    }
    wanted_ver++;
    xSemaphoreGive(lock);
}

bool store_wanted(const char *name) {
    xSemaphoreTake(lock, portMAX_DELAY);
    bool w = is_wanted(name);
    xSemaphoreGive(lock);
    return w;
}

cJSON *store_wanted_with_prefix(const char *prefix) {
    cJSON *a = cJSON_CreateArray();
    size_t pl = strlen(prefix);
    xSemaphoreTake(lock, portMAX_DELAY);
    for (size_t i = 0; i < wanted_count; i++) {
        if (!strncmp(wanted[i], prefix, pl)) {
            cJSON_AddItemToArray(a, cJSON_CreateString(wanted[i]));
        }
    }
    xSemaphoreGive(lock);
    return a;
}

uint32_t store_wanted_version(void) {
    return wanted_ver;
}

void store_mark_all_dirty(void) {
    xSemaphoreTake(lock, portMAX_DELAY);
    for (size_t i = 0; i < count; i++) {
        entries[i].dirty = true;
    }
    xSemaphoreGive(lock);
}

char *store_take_dirty_json(size_t max_bytes) {
    cJSON *o = NULL;
    size_t used = 2;
    xSemaphoreTake(lock, portMAX_DELAY);
    for (size_t i = 0; i < count; i++) {
        entry_t *e = &entries[i];
        if (!e->dirty) {
            continue;
        }
        if (!is_wanted(e->name)) {
            e->dirty = false;  // sent once the deck asks for it (hello marks everything dirty)
            continue;
        }
        size_t need = strlen(e->name) + strlen(e->value) + 8;
        if (o && used + need > max_bytes) {
            break;  // next flush
        }
        if (!o) {
            o = cJSON_CreateObject();
        }
        cJSON_AddStringToObject(o, e->name, e->value);
        used += need;
        e->dirty = false;
    }
    xSemaphoreGive(lock);
    if (!o) {
        return NULL;
    }
    char *s = cJSON_PrintUnformatted(o);
    cJSON_Delete(o);
    return s;
}

cJSON *store_dump_json(const char *prefix, size_t max_bytes, bool *truncated) {
    cJSON *o = cJSON_CreateObject();
    size_t used = 2, pl = strlen(prefix);
    *truncated = false;
    xSemaphoreTake(lock, portMAX_DELAY);
    for (size_t i = 0; i < count; i++) {
        entry_t *e = &entries[i];
        if (strncmp(e->name, prefix, pl)) {
            continue;
        }
        size_t need = strlen(e->name) + strlen(e->value) + 8;
        if (used + need > max_bytes) {
            *truncated = true;
            break;
        }
        cJSON_AddStringToObject(o, e->name, e->value);
        used += need;
    }
    xSemaphoreGive(lock);
    return o;
}
