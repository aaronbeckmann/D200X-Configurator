# ESP32 bridge

An optional hardware mod for the D200X: a Seeed XIAO ESP32S3 on the deck's console UART runs network plugins (Home Assistant, MQTT, HTTP/UDP/Wake-on-LAN, OBS, Spotify, plugins described in its config), so those keys and tiles work without the PC agent. The USB cable between dock and deck is not touched: keystrokes, text and macros keep going to the PC as before.

To deckd, the bridge is a second agent. Plugin, op and state names match the PC agent's, so a deck config works whichever agent serves it. Each plugin's route (`agents.routes` in the deck config) decides which agent is asked first; if that one is down or reports the plugin not ready, the other takes over.

Without the mod everything runs through the PC agent as before. The configurator's "ESP32 bridge..." window leaves the bridge off until "Bridge installed" is ticked.

## Why an ESP32

- Ready under a second after power-up, harmless on power loss (config in NVS flash), small enough to fit inside the case.
- Wi-Fi, TLS, WebSocket and JSON are enough for every network plugin.
- Firmware updates over Wi-Fi with rollback (`docs/WIRING.md` section 7). New integrations need only a config change (`docs/PLUGINS.md`).

## What stays on the PC

Plugins that need the PC itself: `system` (open programs, pages that follow the focused app), `media` (Windows media session), `discord` (Discord's local RPC pipe; there is no network API for your own client's mute/deafen) and `agentdeck`. Only the PC agent offers them, so deckd routes them over USB. The configurator can start the agent at Windows sign-in (top bar, "autostart").

## Files

| Path | Content |
|---|---|
| `docs/WIRING.md` | parts, connections, bring-up, bridge config reference, OTA, troubleshooting |
| `docs/INTERFACE.md` | protocol between deckd and the bridge (frames, session, routing, forwarding) |
| `docs/PLUGINS.md` | plugins described in the bridge config (`${...}` templates) |
| `docs/HARDWARE.md` | teardown facts the mod relies on (console pins, power, dock topology) |
| `main/` | firmware (C, ESP-IDF v6.0.1): `link.c` deck link, `cfg.c` config, `plugin_*.c` plugins, `ota.c` |
| `partitions.csv` | two 3 MB app slots for OTA, `storage` for future plugin scripts |

## Build

In PowerShell, or from the configurator ("ESP32 bridge..." > Build / Flash over USB / Update over Wi-Fi):

```
. C:\esp\v6.0.1\esp-idf\export.ps1
idf.py build
idf.py -p COM<n> flash monitor      # first flash, over the XIAO's USB-C
d200x-tool bridge-ota build\esp32-bridge.bin   # later, over Wi-Fi through the deck
```
