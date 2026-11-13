// Line commands on the XIAO's USB port (idf.py monitor, or any serial terminal):
//   config {"wifi":{...},...}   store a config (one line) and restart
//   status                      print the status document
//   ota http://pc:port/esp32-bridge.bin   install firmware over Wi-Fi
//   reboot
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bridge.h"
#include "driver/usb_serial_jtag.h"
#include "driver/usb_serial_jtag_vfs.h"
#include "esp_system.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#define LINE_MAX_LEN 16384

static void console_task(void *arg) {
    usb_serial_jtag_driver_config_t c = USB_SERIAL_JTAG_DRIVER_CONFIG_DEFAULT();
    c.rx_buffer_size = 2048;
    usb_serial_jtag_driver_install(&c);
    usb_serial_jtag_vfs_use_driver();
    usb_serial_jtag_vfs_set_rx_line_endings(ESP_LINE_ENDINGS_CR);
    setvbuf(stdin, NULL, _IONBF, 0);
    char *line = malloc(LINE_MAX_LEN);
    printf("\nesp32-bridge %s: commands: config {json} | status | ota <url> | reboot\n", BRIDGE_VERSION);
    for (;;) {
        if (!fgets(line, LINE_MAX_LEN, stdin)) {
            vTaskDelay(pdMS_TO_TICKS(100));
            continue;
        }
        line[strcspn(line, "\r\n")] = 0;
        char *cmd = line;
        while (*cmd == ' ') cmd++;
        if (!*cmd) continue;
        if (!strncmp(cmd, "config ", 7)) {
            char err[96] = "";
            if (cfg_save(cmd + 7, err, sizeof err)) {
                printf("config stored, restarting\n");
                link_schedule_restart(500);
            } else {
                printf("config rejected: %s\n", err);
            }
        } else if (!strcmp(cmd, "status")) {
            cJSON *s = link_status_json();
            char *p = cJSON_Print(s);
            printf("%s\n", p ? p : "?");
            free(p);
            cJSON_Delete(s);
        } else if (!strncmp(cmd, "ota ", 4)) {
            char err[96];
            if (!ota_start(cmd + 4, err, sizeof err)) printf("ota: %s\n", err);
        } else if (!strcmp(cmd, "reboot")) {
            esp_restart();
        } else {
            printf("unknown command; use: config {json} | status | ota <url> | reboot\n");
        }
    }
}

void console_start(void) {
    xTaskCreate(console_task, "console", 8192, NULL, 2, NULL);
}
