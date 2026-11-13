# Wiring the ESP32 bridge (Seeed XIAO ESP32S3) into the D200X

Four wires from the deck board to the XIAO. The USB cable between dock and deck stays untouched, so keystrokes keep their path. Background: `HARDWARE.md` (where the pins are), `INTERFACE.md` (protocol), firmware in `..` (this folder's parent, `esp32-bridge/`).

## 1. Parts

| Part | Note |
|---|---|
| Seeed XIAO ESP32S3 (plain or Sense; 8 MB flash, 8 MB PSRAM) with its U.FL antenna | the firmware is built for exactly this board |
| 30 AWG silicone or Kynar wire, about 10 cm per wire | keep the deck-TX wire short: that line is weak (1.85 µs rise time measured) |
| Schottky diode SS14 / 1N5817 (recommended) | in the 5 V wire, so the XIAO's USB-C can be plugged into a PC while it is wired in |
| Kapton tape, a little hot glue or a foam pad | to insulate and fix the XIAO in the case |

## 2. Connections

```
 Deck board (Ulanzi_D200_MB)                          XIAO ESP32S3 (top view, USB-C up)
                                                       ┌──────[USB-C]──────┐
 console TX  (pin next to TP3) ─────────────────────── │ D0  GPIO1    5V   │ ◄── diode cathode
 console RX  (far pin, idles at 3.3 V) ─────────────── │ D1  GPIO2    GND  │ ◄── deck GND
                                                       │ D2           3V3  │   (leave open)
                                                       │ D3           D10  │
                                                       │ D4           D9   │
                                                       │ D5           D8   │
                                                       │ D6 (U0TXD)   D7   │   D6/D7: leave open
                                                       └───────────────────┘
 deck GND   (a GND pin of CN2, or flash pin 4) ──────── GND
 deck 5 V   (the 5 V pin of CN2) ── SS14 ──|>|──────── 5V      (anode at the deck, cathode band at the XIAO)
```

| Deck | Direction | XIAO pin | GPIO | Function |
|---|---|---|---|---|
| console TX, pin next to `TP3` (the one that showed the `UUUU` pattern) | deck → XIAO | **D0** | GPIO1 | UART1 RX |
| console RX, the far pin (steady 3.3 V) | XIAO → deck | **D1** | GPIO2 | UART1 TX |
| GND | | **GND** | | common ground |
| 5 V (the USB 5 V pin of `CN2`, as measured in `HARDWARE.md`) | deck → XIAO | **5V** (through the diode) | | power |

Rules:
- **Do not use D6/D7.** They are UART0; the ESP32 ROM prints its boot log there at every reset, and the deck console is a root shell whenever deckd is not running.
- **Do not connect the XIAO's 3V3 pin** to anything on the deck. The XIAO makes its own 3.3 V from 5 V; Wi-Fi peaks (up to about 500 mA) must not load the deck's 3.3 V rail.
- **Both sides are 3.3 V logic.** Direct connection, no level shifter.
- **Before the first power-up**, check with a multimeter: no short between 5 V and GND, and the TX/RX wires go to D0/D1 the right way round (deck TX to D0).

## 3. Placement

- The XIAO ESP32S3 has no antenna on the board: plug the supplied flat U.FL antenna into its connector before the first Wi-Fi start. Stick the antenna to a plastic inside wall of the case, away from the shielding tape over the deck board and from the dock's metal parts.
- Insulate the XIAO's underside with Kapton tape before laying it on the board.
- Route the USB-C of the XIAO so it can still be reached after reassembly, if possible: flashing and the config console use it.

## 4. Bring-up, step by step

All deck-side steps run deckd from `/tmp` (nothing flashed) until step 6.

1. **Flash the XIAO on the desk** (not wired yet). In PowerShell:
   ```
   . C:\esp\v6.0.1\esp-idf\export.ps1
   cd "C:\Users\aaron\Desktop\Ulanzi D200X\esp32-bridge"
   idf.py build
   idf.py -p COM<n> flash monitor
   ```
   (`COM<n>`: the XIAO's USB port in Device Manager. If it does not show up, hold BOOT, tap RESET, release BOOT.)
   The monitor prints `listening on UART1 RX=GPIO1 (D0), TX=GPIO2 (D1) held off` and `no config stored`.

2. **Configure Wi-Fi and plugins** in the monitor, one line (see section 6 for all options):
   ```
   config {"wifi":{"ssid":"MyWifi","password":"..."},"homeassistant":{"url":"http://homeassistant.local:8123","token":"..."}}
   ```
   It restarts and logs `wifi: connected, ip ...` and `plugin homeassistant enabled`. `status` prints everything (secrets as `***`).

3. **Receive path only.** Unplug the XIAO. Solder **GND** and **D0 ← deck console TX** only (no D1, no 5 V), and power the XIAO from the PC's USB. Run a dry run of the new deckd with the UART link:
   ```
   powershell -ExecutionPolicy Bypass -File research\adb-dryrun.ps1 -Uart
   ```
   (then replug the deck when the script asks). The XIAO monitor must print `deck: deckd 0.1.0, link up`. With D1 not wired, the deck cannot hear the XIAO yet, so the deck shows no agent; this step proves the receive side without the XIAO ever driving the deck's console.

4. **Both directions.** Unplug, add **D1 → deck console RX**. Repeat step 3. Now:
   - XIAO monitor: `deck: deckd ..., link up`, `deck: hello, N states wanted`, `ha: connected`, `ha: watching N entities`.
   - Deck status line: `esp` (or `pc+esp` with the PC agent running).
   - `d200x-tool log` (PC): `agent connected on esp: esp32-bridge 0.1.0 [...]`.
   - `d200x-tool bridge-status` (PC): the XIAO's status JSON through the deck.

5. **Power from the deck.** Unplug, add the diode and the **5 V** wire. Unplug the XIAO's USB-C, replug the deck: the XIAO now starts with the deck. LED: off = no deckd beacon, blinking = deck linked but no Wi-Fi, on = linked and online.

6. **Make it permanent.** In the configurator: Firmware... > Install deckd (the image now contains the bridge link), then **ESP32 bridge...** > tick "Bridge installed", pick the plugins to run on the bridge, enter Wi-Fi, "Send settings to the bridge", then "Save to deck". The same window builds the bridge firmware and flashes it over USB or updates it over Wi-Fi. By hand instead: add the agents section to the deck config (configurator JSON or `d200x-tool get-config` / edit / `set-config`):
   ```json
   "agents": { "uart": { "enabled": true } }
   ```
   then build and flash the image that contains the new deckd (`d200x-device/image/mkimage.py`, then the configurator's Firmware window or `research/adb-install.ps1`). Turning `enabled` off again closes the link and restarts the console shell at once.

## 5. What runs where

- Keys with keyboard actions: the deck itself, always (USB HID to the PC; untouched).
- `host` actions: deckd sends each to the agent whose route comes first and which reports the plugin ready. Default route for every plugin: ESP32 first, then the PC agent. PC-only plugins (`system`, `media`, `discord`, `agentdeck`) are only offered by the PC agent, so they go there automatically. Override per plugin in the deck config: `"agents": {"uart": {"enabled": true}, "routes": {"homeassistant": ["usb", "uart"]}}`.
- If the ESP32's Home Assistant connection drops, the plugin reports not ready and the PC agent takes over (if it runs and has HA configured); states follow the same owner.

## 6. Bridge config reference

Set with `config {...}` on the XIAO's USB console, or through the deck with `d200x-tool bridge-config bridge.json`. A status read-back shows secrets as `***`; sending such a document back keeps the stored secrets.

```json
{
  "wifi": { "ssid": "MyWifi", "password": "..." },
  "homeassistant": { "url": "http://homeassistant.local:8123", "token": "<long-lived access token>" },
  "mqtt": { "uri": "mqtt://192.168.1.2:1883", "username": "deck", "password": "...",
            "subscriptions": { "home/door/state": "door", "sensors/+/temp": "temp" } },
  "net": { "polls": [ { "state": "outside", "url": "http://192.168.1.2/api/temp", "pointer": "/value", "every_s": 60 } ] },
  "connections": { "nas": { "url": "http://192.168.1.5:5000", "headers": { "Authorization": "Bearer ..." } } },
  "obs": { "url": "ws://192.168.1.10:4455", "password": "..." },
  "spotify": { "client_id": "<from the configurator's Spotify setup>", "refresh_token": "<from agent.json after the PKCE login>" }
}
```

Deck key examples (deck config, `action`):

```json
{ "type": "host", "plugin": "homeassistant", "op": "toggle", "args": { "entity_id": "light.desk" } }
{ "type": "host", "plugin": "mqtt", "op": "pub", "args": { "topic": "home/scene", "payload": "movie" } }
{ "type": "host", "plugin": "net", "op": "http", "args": { "conn": "nas", "path": "/api/backup", "method": "POST" } }
{ "type": "host", "plugin": "net", "op": "wol", "args": { "mac": "aa:bb:cc:dd:ee:ff" } }
{ "type": "host", "plugin": "net", "op": "udp", "args": { "host": "192.168.1.40", "port": 4210, "payload": "on" } }
{ "type": "host", "plugin": "obs", "op": "scene", "args": { "name": "Camera" } }
{ "type": "host", "plugin": "spotify", "op": "play_pause" }
```

States for tiles: `ha:<entity>[:text|:name|:attr:<a>]`, `mqtt:<name>`, `net:<state>`, `obs:scene|recording|streaming|virtualcam|connected|mute:<input>`, `spotify:playing|title|artist|album|line|shuffle|repeat|liked|volume|device|connected`.

New integrations without new firmware: plugins described in the config, see `PLUGINS.md`.

## 7. Updating the bridge firmware over Wi-Fi

After the first USB flash, the XIAO never needs its USB port again:

```
cd esp32-bridge && idf.py build
d200x-tool bridge-ota esp32-bridge/build/esp32-bridge.bin
```

The tool serves the image from the PC on the network interface that reaches the bridge, the bridge downloads it into its second app slot (progress in `d200x-tool log`), restarts, and the tool prints the new version. Allow `d200x-tool` in the Windows Firewall prompt (private network). Safety: the new image confirms itself once Wi-Fi is up (or after 60 s without a crash); if it crashes or hangs before that, the next reset boots the previous image. The config in NVS is kept. On the USB console the same works with `ota http://<pc>:<port>/file.bin`; `d200x-tool bridge-reboot` restarts the bridge, `d200x-tool bridge-states` lists every state it knows (firmware 0.3.0 and later).

The first flash over USB must use this partition layout (two app slots, `storage` for future plugin scripts). A XIAO flashed with the earlier single-slot build needs one more USB flash (`idf.py -p COM<n> flash`), which also keeps the config.

## 8. Troubleshooting

| Symptom | Check |
|---|---|
| XIAO never prints `link up` | deckd runs with `--uart` (dry run `-Uart`) or `agents.uart.enabled`; D0 on the pin next to `TP3`; GND connected; `d200x-tool log` shows `uart: /dev/ttyS0 open at 115200` |
| `link up` but the deck shows no agent | D1 wiring (to the far pin); `d200x-tool log` for `uart: N bad frames` |
| deck log `uart: console shell had come back` | init restarted the console shell; deckd kills it again (checks at 2, 5, 10 and 30 s). Report if it keeps happening |
| plugin not ready in `bridge-status` | Wi-Fi (`wifi.connected`), URL and token; the XIAO monitor shows the reason (`ha: token rejected`, `spotify: token refresh failed`) |
| Spotify `auth` failures after weeks | the refresh token was revoked: log in again in the configurator and send the new token with `bridge-config` |
