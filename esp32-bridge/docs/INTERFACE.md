# ESP32 bridge: interface specification (deck side and ESP32 side)

Status: implemented and running on hardware (XIAO ESP32S3 wired to the console UART, 2026-11-02). Wiring and bring-up: `WIRING.md`. Overview: `../README.md`.

| Part | Where | Differences from this spec |
|---|---|---|
| deckd UART link, agent table, routing, state ownership, beacon, forwarding | `d200x-device/deckd/src/{frame,uart,main}.rs` | baud change (`0x0214`) is answered with the current rate only |
| config `agents` section, `plugin_of_state` | `deck-config` | |
| `bridge-config` / `bridge-status` / `bridge-ota` | `d200x-core`, `d200x-tool`, configurator window "ESP32 bridge..." (`d200x-configurator/src/ui_bridge.rs`) | |
| ESP32 firmware | `esp32-bridge/` (C on ESP-IDF v6.0.1, XIAO ESP32S3) | C instead of Rust (no Xtensa Rust toolchain installed), so there is no shared `deck-link` crate: the frame code exists twice (`frame.rs`, `link.c`) and both pass the same test cases. No captive portal: config over the USB console or through the deck. No `hid` plugin yet. `uart-sim` not built |
| Config-defined plugins | `esp32-bridge/main/plugin_decl.c`, reference `PLUGINS.md` | addition to this spec: HTTP/MQTT/UDP/WoL templates with `${...}` expressions, polled and MQTT states |
| Over-the-air update | `esp32-bridge/main/ota.c`, `partitions.csv` (two 3 MB app slots, rollback, 1.9 MB `storage`), `d200x-tool bridge-ota` | addition: command `0x0224` (USB) -> `0x0324` (UART) `{"req","cmd":"ota","url"}` / `{"cmd":"reboot"}` / `{"cmd":"states","prefix"?}` (every state the plugins know, wanted or not, up to 12 KB, with `"plugins"` and `"truncated"`; the configurator's state explorer polls it every 10 s), reply `0x0225` -> `0x0325`. deckd forwards the whole range `0x0220..=0x022f` (even = request, odd = reply), so later bridge commands need no deckd change |

Design date: 2026-11-02. Hardware facts in `HARDWARE.md` (console TX next to `TP3`, RX on the far pin, confirmed with a scope), the board choice in `../README.md`, the existing agent protocol in `../../docs/PLUGINS-AND-HOST-AGENT.md` 3.2 and `../../d200x-core/src/protocol.rs`.

Principle: **the ESP32 is a second agent.** deckd already talks to one agent (the PC) over the vendor HID channel with hello / heartbeat / states / host action / result. The same messages run over the console UART. deckd gains a second link and routes each host action to the agent that can serve it. Plugin names, op names and state names are identical on both agents, so a deck config works unchanged whichever agent serves it.

```
            USB vendor HID (/dev/hidg0)                       UART (/dev/ttyS0, 115200 8N1)
PC agent  <───────────────────────────>  deckd  <────────────────────────────────────────>  ESP32-S3  ── Wi-Fi ──> HA, MQTT, HTTP, OBS, Spotify
(system, media, discord, agentdeck,       │                                                   (homeassistant, mqtt, net, obs,
 homeassistant, obs, spotify)             └─ keyboard reports (/dev/hidg1) to the PC,          spotify, hid)
                                             unchanged, never via an agent
```

## 1. Physical layer

| Item | Value |
|---|---|
| Deck side | `/dev/ttyS0` (SoC console UART). TX = pin next to `TP3`, RX = far pin (pulled up, idle 3.3 V) |
| ESP32 side | UART1 on two free, non-strapping GPIOs (S3: avoid 0, 3, 45, 46; avoid 19/20 = USB) |
| Levels | 3.3 V both sides, direct connection, common GND |
| Format | 115200 baud, 8N1, no flow control |
| Rate limit | Deck TX rise time measured 1.85 µs: 115200 is safe, higher rates need a 74LVC1G17 buffer at the tap (then 460800 or 921600, negotiated in the beacon, section 3.1) |
| Power | ESP32 module from the deck's 5 V (USB VBUS at `CN2`) through its own 3.3 V regulator |
| ESP32 TX at boot | GPIO configured as **input (high impedance) until the first valid deck beacon** (section 3.1). The deck's RX pull-up keeps the line idle |

Why the TX gating matters: while deckd is not running (boot, ADB window, crash), `/dev/ttyS0` belongs to `service console /bin/sh`, a **root shell**. Any byte the ESP32 sends then is shell input.

## 2. Frame layer

```
offset  size  field
0       2     SOF = 7c 7c
2       2     cmd, u16 big-endian            (same numbers as the HID channel)
4       4     len, u32 little-endian         (payload bytes; same as the HID header)
8       len   payload                        (UTF-8 JSON unless stated otherwise)
8+len   2     crc, u16 big-endian            CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF, no reflection,
                                             no final XOR) over bytes 2 .. 8+len (cmd, len, payload)
```

- The header is the HID channel's header, so `d200x-core::protocol` keeps one set of command numbers. Differences from HID: no 1024-byte reports and no padding, and a CRC trailer.
- `len` must be at most **16384**. Larger values are treated as corruption.
- **Receiver state machine**: hunt for `7c 7c`; read 6 header bytes; check `len`; read payload and CRC; on CRC mismatch, oversize length, or more than 50 ms between bytes inside a frame, drop the frame and resume hunting **at the byte after the SOF that started it** (so a real SOF inside garbage is found). Bytes outside frames are ignored; this absorbs IPL/U-Boot output (about 20 bytes at boot) and line noise.
- No retransmission at this layer. Losses are covered above it: heartbeats repeat, the deck re-sends hello on reconnect, states are re-sent as a snapshot after hello, host actions time out (15 s) and flash red.
- Both sides **write whole frames from one writer** (a queue + writer thread/task), so frames never interleave.
- Test vector: cmd `0x0210`, payload `{}` → `7c 7c 02 10 02 00 00 00 7b 7d` + CRC over `02 10 02 00 00 00 7b 7d`. The shared crate (section 6) holds the computed value and unit tests for both implementations.

## 3. Session layer

### 3.1 Start-up, liveness, deck restart

1. **deckd starts**, opens the UART (section 4.2), then sends a **beacon** `0x0312` every 1 s:
   `{"firmware":"deckd","version":"0.x","boot":"<random u32 hex per deckd start>","uptime_ms":1234,"baud":[115200]}`
   `baud` lists the rates deckd accepts (only 115200 until a buffer is fitted).
2. **ESP32** listens with TX high-Z. On the first valid beacon it enables TX and sends a heartbeat `0x0210` at once, then every 2 s.
3. **deckd**, on the first heartbeat from the UART link, sends hello `0x0311` on that link (the same JSON as on USB: `firmware`, `version`, `serial`, `states` (wanted), `icons`).
4. **ESP32**, on hello, stores the wanted list, (re)subscribes its plugins, and sends a **full snapshot** of every wanted state it owns (section 3.4) as one or more `0x0211` frames.
5. **Liveness**: deckd treats the ESP32 as gone after 6 s without a heartbeat (as for the PC agent). The ESP32 treats the deck as gone after 3 s without a beacon and **returns its TX to high-Z** (deckd stopped or the deck rebooted, so the console may be a shell again).
6. **Deck restart detection**: a beacon whose `boot` differs from the last one means deckd restarted: the ESP32 waits for the new hello, then sends a full snapshot again.
7. **Optional rate change** (only with the buffer fitted): the ESP32 asks with `0x0214 {"baud":921600}`; deckd answers `0x0314 {"baud":921600}` at the old rate, both switch after 100 ms, and fall back to 115200 if no beacon arrives within 3 s.

### 3.2 Messages

Deck → agent (numbers `0x03xx`; the first two exist today):

| cmd | name | payload | when |
|---|---|---|---|
| `0x0310` | host action | `{"id":17,"plugin":"homeassistant","op":"toggle","args":{...},"event":"press"\|"turn","delta":-1,"page":"main"}` | key, knob or side-button action routed to this agent |
| `0x0311` | hello | `{"firmware","version","serial","states":[...],"icons":[...]}` | first heartbeat on a link, config change, route change |
| `0x0312` | beacon | section 3.1 | every 1 s, **UART only** |
| `0x0314` | baud ack | `{"baud":n}` | reply to `0x0214` |
| `0x0320` | agent config (forwarded) | `{"req":5,"config":{...}}` | PC configurator -> deckd -> ESP32 (section 3.5) |
| `0x0322` | agent status request (forwarded) | `{"req":6}` | PC configurator -> deckd -> ESP32 |

Agent → deck (numbers `0x02xx`; the first three exist today):

| cmd | name | payload | when |
|---|---|---|---|
| `0x0210` | heartbeat | `{"agent":"esp32-bridge 0.1.0","plugins":["homeassistant","mqtt","net"],"ready":{"homeassistant":true,"mqtt":false,"net":true},"prefixes":{"ha":"homeassistant","mqtt":"mqtt","net":"net"}}` | every 2 s |
| `0x0211` | states | `{"ha:light.kitchen":"on","ha:sensor.temp:text":"21.5 °C"}` (partial map) | snapshot after hello, then changes |
| `0x0212` | result | `{"id":17,"ok":false,"error":"timeout"}` | once per host action, at most 10 s after it |
| `0x0213` | log | `{"level":"warn","msg":"ha: reconnecting"}` | important events only; deckd appends them to its log tail as `esp32: ...` (visible in `d200x-tool log`) |
| `0x0214` | baud request | `{"baud":n}` | optional, section 3.1 |
| `0x0221` | agent config reply | `{"req":5,"ok":true,"error":null}` | answer to `0x0320` |
| `0x0223` | agent status reply | `{"req":6,"wifi":{"ssid","rssi","ip"},"plugins":{...},"heap_free":..,"psram_free":..,"version":..,"uptime_s":..}` | answer to `0x0322` |

New heartbeat fields (`ready`, `prefixes`) are optional: the PC agent's current heartbeat (`agent`, `plugins`) stays valid, with defaults "every listed plugin is ready" and the built-in prefix table (`ha` -> `homeassistant`, `obs`, `spotify`, `discord`, `media`, `system`, `agentdeck`).

### 3.3 Routing host actions (deckd)

- For a `Host { plugin, .. }` action deckd picks the link by the config's route list for that plugin (section 4.1, default `["uart","usb"]`): the first link whose agent is alive, lists the plugin, and reports it `ready`.
- No such link: run `fallback`, else flash red (today's behaviour).
- The pending entry records the link. Results are matched by `id` only. A result that has not arrived after 15 s means a red flash (as today).
- Knob turns are sent per detent as today. The ESP32 coalesces bursts per entity (for example brightness steps arriving within 150 ms become one service call).

### 3.4 State ownership

- Each state name has a prefix (`ha:`, `mqtt:`, `net:`, `obs:`, ...), and each prefix belongs to a plugin (heartbeat `prefixes`, or the built-in table).
- The **owner** of a prefix is the agent that deckd would route that plugin's actions to (section 3.3). deckd accepts `0x0211` entries only from the owner and drops the others, so a PC agent and an ESP32 that both run Home Assistant never fight over `ha:*`.
- When the owner changes (agent lost, plugin not ready, agent back), deckd sends hello to the new owner, which answers with a full snapshot. Values from the old owner stay displayed until replaced, so the deck does not flicker to "-".
- Agents only send states from the hello's wanted list that fall under their own prefixes. This keeps the UART budget (about 11 KB/s) for what is on screen.
- Agents coalesce changes: at most 10 `0x0211` frames per second, each at most 2 KB. A snapshot is split into several frames.

### 3.5 Configuring the ESP32 through the deck (no secrets on the deck)

- The configurator sends `0x0220 {"req":n,"target":"uart","config":{...}}` to deckd over USB. deckd forwards it to the UART agent as `0x0320`, keeping it only in RAM, and relays the ESP32's `0x0221` reply back to USB as `0x0321` (deck-to-host numbers are `0x03xx` on USB, as everywhere else). Likewise for status: `0x0222` (USB) -> `0x0322` (UART) -> `0x0223` (UART) -> `0x0323` (USB). No ESP32 on the link: deckd answers `{"req":n,"ok":false,"error":"no uart agent"}`.
- The ESP32 config holds Wi-Fi credentials, connections (`homeassistant`: url + token; `mqtt`: uri, user, password; `obs`: url + password; `spotify`: client id + refresh token), MQTT subscriptions (topic -> state) and HTTP polls (url, period, JSON pointer -> state). The ESP32 stores it in NVS and returns it redacted (secrets replaced by `"***"`) in status replies.
- Fallback without a deck: a captive-portal page from the ESP32 when it has no Wi-Fi configured, or when the BOOT button is held at power-up.

## 4. Deck side (deckd)

### 4.1 Config model (`deck-config`)

```json
"agents": {
  "uart": { "enabled": true, "baud": 115200 },
  "routes": { "homeassistant": ["uart", "usb"], "agentdeck": ["usb"] }
}
```

- New optional `Config.agents` (serde default: UART disabled, so current configs and decks without an ESP32 behave as today). `routes` defaults to `["uart","usb"]` per plugin: the always-on ESP32 first, the PC as fallback.
- `wanted_states()` is unchanged. The prefix table moves into `deck-config` so deckd, the configurator and both agents share it.

### 4.2 UART link (`deckd/src/uart.rs`, new)

`SerialLink` with the same surface as `hid::Vendor`: `open() / closed() / is_open() / fd() / poll() -> Vec<Message> / send(cmd, payload)`.

Opening, in order:
1. `setprop ctl.stop console`, then scan `/proc/*/fd/*` for links to `/dev/ttyS0` and kill those processes (the console shell). Log what was stopped.
2. `open("/dev/ttyS0", O_RDWR | O_NOCTTY | O_NONBLOCK)`; termios: `cfmakeraw`, `CLOCAL | CREAD`, 8N1, no `CRTSCTS`, 115200, `VMIN 0 / VTIME 0`; `ioctl(TIOCEXCL)`; `tcflush(TCIOFLUSH)`.
3. Keep the kernel off the console: `echo 0 > /proc/sys/kernel/printk` (the cmdline already has `loglevel=0`).
4. Re-check after 2 s that no shell holds the tty (init may respawn it); if one does, kill it again and log the respawn. On the unit tested, init did not respawn it while deckd held the link.

Writer: a thread fed by a bounded `sync_channel(32)`, which writes whole frames and drops them (with a counter) while the queue is full, as `Vendor` does. Reader: the non-blocking fd joins the main loop's `poll()` set next to the MCU and the HID fd. The frame parser is the shared crate's (section 6).

### 4.3 Agent table (`deckd/src/main.rs`)

Replace `last_heartbeat` / `agent_name` with one entry per link:

```rust
enum LinkId { Usb, Uart }
struct AgentInfo { name: String, plugins: Vec<String>, ready: HashMap<String, bool>,
                   prefixes: HashMap<String, String>, last_heartbeat: Instant }
agents: [Option<AgentInfo>; 2]          // indexed by LinkId
fn agent_alive(&self, l: LinkId) -> bool
fn route(&self, plugin: &str) -> Option<LinkId>
fn owner_of(&self, state: &str) -> Option<LinkId>
```

- `handle_host(msg)` becomes `handle_host(link, msg)`. Config, icon, log, calibrate and reboot commands stay **USB only** (the ESP32 cannot change the deck's config). Agent messages (`0x0210..0x0214`, `0x0221`, `0x0223`) are accepted per link.
- `run_action_from` sends `0x0310` on `route(plugin)`; `pending` stores `(id, bit, link, sent_at)`.
- `0x0211` handling filters by `owner_of`.
- Beacon timer (1 s) in the main loop while the UART is open.
- Status line: `pc`, `esp`, `pc+esp` or `no agent` instead of the single dot. Tiles whose state prefix has no owner keep their last value.
- Hello goes to every alive agent on config change, and to a new prefix owner on an owner change.
- Command-line flag `--uart` (or the config's `agents.uart.enabled`); `--uart-echo` test mode that answers every frame with the same frame (for link tests from `/tmp`).

### 4.4 Rollout

1. Shared crate and unit tests on the PC.
2. deckd from `/tmp` (no flash) with `--uart-echo`, a USB-serial adapter on the PC playing the ESP32 (section 7).
3. deckd from `/tmp` with the ESP32 firmware.
4. Rebuild `update.img`, flash.

## 5. ESP32 side (firmware)

Target: ESP32-S3 with PSRAM (N16R8). Language: Rust on ESP-IDF (`esp-idf-svc`, std, threads), which lets it share the frame and message crate with deckd and the PC agent. C++ on ESP-IDF works the same way if preferred.

### 5.1 Tasks

| Task | Job |
|---|---|
| `link_rx` | UART1 driver (RX ring buffer 4 KB), shared frame parser, dispatches messages |
| `link_tx` | queue -> frames; owns the TX pin; switches it to output only while the session is up |
| `session` | beacon tracking, heartbeat every 2 s, hello handling (wanted list, re-subscribe), state flushing (100 ms, coalesced, ≤ 2 KB frames), result timeouts (10 s) |
| `wifi` | station mode with reconnect backoff; captive-portal provisioning when unconfigured |
| one per plugin | network client and its reconnect loop; feeds the state store; executes ops from its own queue |
| `hid` (S3, optional) | TinyUSB keyboard + consumer control on the native USB port |

### 5.2 Plugin interface

```rust
trait Plugin: Send {
    fn name(&self) -> &'static str;                 // "homeassistant", "mqtt", "net", "obs", "spotify", "hid"
    fn prefix(&self) -> &'static str;               // "ha", "mqtt", "net", "obs", "spotify", ""
    fn ready(&self) -> bool;                        // connected and authenticated
    fn set_wanted(&mut self, states: &[String]);    // from hello: subscribe only to these
    fn run(&mut self, op: &str, args: &Value, event: &str, delta: i32) -> Result<()>;
}
```

The state store is a map `name -> value` with a dirty set. Plugins write into it, and the session task flushes the dirty, wanted entries.

### 5.3 Plugins and their ops

Names and ops match the PC agent where the plugin exists there, so configs move between agents unchanged.

| plugin | ops | states | notes |
|---|---|---|---|
| `homeassistant` | `toggle`, `turn_on`, `turn_off`, `call_service`, `scene`, `script`, `trigger`, `brightness`, `volume`, `media`, `temperature`, `number`, `select_next` (as `d200x-agent/src/ha.rs`) | `ha:<entity>`, `:text`, `:name`, `:attr:<a>` | WebSocket with `subscribe_entities` **filtered to the wanted entity ids** (never the full `get_states`: 689 entities, about 350 KB) |
| `mqtt` | `pub` {topic, payload, retain, qos} | `mqtt:<name>` from configured subscriptions | ESP-IDF `esp-mqtt`, TLS optional |
| `net` | `http` {method, url \| conn+path, headers, body, `state` + `pointer`}, `wol` {mac}, `udp` {host, port, payload \| hex} | `net:<name>` from configured polls and `http` results | response bodies capped at 64 KB (PSRAM) |
| `obs` | as the PC plugin (scenes, record/stream/virtualcam toggles, mute, source visibility, studio mode) | `obs:*` as the PC plugin | obs-websocket v5 over the LAN, SHA-256 auth |
| `spotify` | `play_pause`, `next`, `prev`, `shuffle`, `repeat`, `like`, `volume`, `play` {uri} | `spotify:*` as the PC plugin, no album art | refresh token from the PC login, stored in NVS |
| `hid` | `media` {usage: play_pause, next, prev, mute, vol_up, vol_down}, `key` {keys}, `text` {text} | none | S3 native USB on a freed GL850S port; only with that wiring |

### 5.4 Resource budget (S3 N16R8)

TLS sessions about 40 KB of internal RAM each (HA, MQTT, Spotify, and one transient for `net`), JSON parsing and HTTP bodies in PSRAM, UART buffers 4 KB RX + 4 KB TX, the state store a few hundred entries. Comfortable; a C3 without PSRAM would need to drop to two TLS sessions.

### 5.5 Failure behaviour

- Plugin disconnected: `ready=false` in the next heartbeat. deckd routes to the PC agent if it serves that plugin, otherwise falls back or flashes red.
- Wi-Fi down: all network plugins become not ready, while the heartbeat continues (the link itself is fine).
- ESP32 reset: the deck sees heartbeats stop, then a new session starts at the next beacon.
- Watchdog: ESP-IDF task watchdog on `link_rx` and `session`; a plugin task that hangs is restarted by its supervisor loop.

## 6. Frame code on both sides

The firmware is C on ESP-IDF (no Xtensa Rust toolchain), so there is no shared crate: the frame layer exists twice, `d200x-device/deckd/src/frame.rs` and `esp32-bridge/main/link.c`, with the same behaviour (CRC check value `"123456789"` -> `0x29B1`, resync, look-ahead past false starts such as `|||` in boot text, oversize `len`, 1 s stall). The Rust side carries the tests (`rustc --test src/frame.rs`); the message numbers live in `d200x-core::protocol` and `deckd`.

## 7. Test tools

- `research/adb-uarttest.ps1`: the low-level check for the deck's TX (pattern on `/dev/ttyS0`).
- `research/adb-dryrun.ps1 -Uart`: runs a new deckd from `/tmp` with the UART link, nothing flashed.
- `d200x-tool log`, `d200x-tool bridge-status`: link and bridge state through the deck.
- The XIAO's USB console (`idf.py monitor`): `status`, `config {...}`, `ota <url>`, `reboot`.

## 8. Settled points

1. The console shell: deckd stops it with `setprop ctl.stop console`, kills any `sh` still holding the tty, and re-checks at 2, 5, 10 and 30 s. No respawn seen on the tested unit.
2. Pins and mounting: XIAO ESP32S3, UART1 on D0/D1 (GPIO1/GPIO2), see `WIRING.md`. No USB HID wiring to a GL850S port.
3. Rate: 115200, no buffer (1.85 µs rise time). `0x0214` baud change is answered with the current rate only.
4. Route defaults: `["uart","usb"]` for every plugin; the configurator sets Home Assistant, OBS and Spotify per its "run on the bridge" choices. `agentdeck`, `system`, `media` and `discord` exist only on the PC agent and route to USB automatically.
