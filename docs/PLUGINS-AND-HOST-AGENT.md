# Ulanzi Studio features vs. deckd: what runs on the deck, what needs the PC, and the Home Assistant design

Date: 2026-10-30. Basis: the plugin manifests installed in Ulanzi Studio 3.4.4 on this machine, the deckd firmware as of today, and the hardware facts in `../INVESTIGATION.md`.

## 1. The hardware boundary

- The deck's Linux has one network interface: `lo`. No Wi-Fi, no Ethernet, and the vendor USB gadget driver (`zkswe_usb`) only implements `adb` and `hid`; the kernel contains no NCM/RNDIS/ECM function. Changing that means a kernel rebuild from SigmaStar's SDK, which is out of scope.
- Therefore: everything that touches the network or a PC application needs an agent on the PC. This is the same model as Ulanzi Studio (which must run for OBS, Spotify, Home Assistant, ...) and as Elgato's software. deckd's difference: the deck owns its configuration and executes all keyboard-type actions itself, so the deck is never dead when the PC software is not running.
- The deck can do locally: USB keyboard reports (any key combination, typed text, macros, timing), pages, visual state, timers, brightness. It cannot do locally: launch programs, open websites, play sounds on the PC, talk to OBS/Discord/Spotify/Home Assistant/HTTP APIs, or read PC state.

## 2. Ulanzi Studio plugin inventory and the deckd equivalent

| Ulanzi plugin / action | Needs PC? | deckd equivalent | Status |
|---|---|---|---|
| System: Hotkey, Text | no | `key`, `text` actions | done |
| System: Multimedia (play/pause, next, ...) | no | keyboard-page usages: mute / volume up / volume down work from the keyboard page; play/pause/next/prev are *consumer-page* usages, which the stock USB descriptor does not include | partial: volume + mute now; transport keys need either a descriptor change (kernel driver, hard) or the host agent (easy) |
| System: Hotkey Switch (2 states) | no | `toggle` action with two sub-actions and two looks | to do (small) |
| System: Open, Close, Website | yes | host action `open`, `close`, `url` | agent, phase 3 |
| Multi Action, Delay, Random, Multi Action Switch | no | `macro` (done); `random` and 2-state switch | to do (small) |
| Pages: prev/next/goto/parent/subpage/indicator/profile switch | no | pages + side buttons (done); subpage = page with a "back" key (done by config); page indicator = status line | mostly done; "profiles" = separate config files in the configurator |
| Action Trigger (combo / trigger) | no | long-press and double-press variants per key | to do (device: MCU gives press/release timing) |
| POP (feature popup) | no | not needed | skip |
| Soundboard | yes | host action `sound` (play file on PC) | agent, phase 3 |
| Dial: offline mode, combined action | no | knob press/left/right actions per page (done); acceleration (steps per detent) | done / to do |
| Ulanzi Connect (BLE lights) | yes (PC Bluetooth) | host plugin `ulanzi-light` | later, low priority |
| OBS Studio (19 actions) | yes | host plugin `obs` via obs-websocket v5 | agent, phase 3 |
| TC002 | yes | host plugin via the clock's HTTP API | later |
| Home Assistant: Entity | yes | host plugin `homeassistant` | **phase 2, the goal** |
| Discord, Spotify, API Request, GitHub stats, AgentDeck | yes | host plugins | phase 3 / as wanted |

## 3. Architecture: host actions, state and looks

### 3.1 Config model additions (shared `deck-config`)

```json
"action": { "type": "host", "plugin": "homeassistant", "op": "toggle", "args": { "entity_id": "light.kitchen" },
            "fallback": { "type": "none" } }
"looks": { "on":  { "icon": "mdi-lightbulb-on.png", "color": "#f5b400", "label": "Kitchen" },
           "off": { "icon": "mdi-lightbulb-outline.png", "color": "#2a3440", "label": "Kitchen" } }
"state": "homeassistant:light.kitchen"
"value": "homeassistant:sensor.living_temp"
```

- `host` action: the deck sends an event to the agent and does nothing else. If no agent has been heard from within 5 s, the key flashes red and runs `fallback` if present (so a light key can fall back to nothing, a media key to a keyboard usage).
- `looks`: a map from state name to a look (icon, colour, text colour, label). The deck keeps a state table `{name: value}`; a key whose `state` resolves to a value with a matching look shows that look. Unknown or missing state: the base look.
- `value`: a state whose value is displayed as the label (sensor readings, song title, scene name). The agent pushes strings already formatted ("21.5 °C").
- `toggle`: device-side two-state action (hotkey switch) with looks `a`/`b`; no agent involved.

### 3.2 Protocol additions (vendor HID channel, stock framing)

Device -> host (events, seq 0 like the stock input reports):
- `0x0310` host action: `{"id": 17, "key": "main/4", "plugin": "homeassistant", "op": "toggle", "args": {...}, "event": "press"|"release"|"turn", "delta": -1}` (knobs send `turn` with a delta, acceleration applied on the device).
- `0x0311` hello: `{"firmware":"deckd","serial":...,"states_wanted":[...]}` sent at start and whenever the config changes, so the agent knows which states and values to feed.

Host -> device:
- `0x0210` agent heartbeat: `{"agent":"d200x-agent 0.1","plugins":["homeassistant","system"]}` every 2 s. The deck shows a small dot in the status line while the agent is alive.
- `0x0211` states: `{"homeassistant:light.kitchen":"on", "homeassistant:sensor.living_temp":"21.5 °C"}` (partial updates).
- `0x0212` result: `{"id":17,"ok":false,"error":"..."}` -> red flash on failure.
- `0x0201` icon upload (existing) is used by the agent to push rendered icons on demand; the deck answers which icons it already has in the hello so uploads happen once.

### 3.3 The agent (`d200x-agent`, Rust)

- Runs on the PC as a tray application (and inside the configurator while it is open; same crate). Autostart optional. Reconnects to the deck whenever it re-enumerates; holds interface 0, so Ulanzi Studio must not run.
- Plugin trait: `fn ops(&self) -> &[OpSpec]`, `fn run(&mut self, op, args) -> Result`, `fn subscribe(states) -> stream of (state, value)`, `fn icon_for(entity) -> Option<IconSpec>`. Plugins: `system` (open/url/close/sound/media keys via the PC), `homeassistant`, later `obs`, `spotify`, `discord`, `http`.
- The configurator uses the same plugin code to offer pickers (HA entity list, OBS scene list) and to generate icons.

### 3.4 Application pages (focus-based switching)

- `system` plugin states `system:app` (lowercase process image name of the foreground window, e.g. `code.exe`) and `system:title` (window title), polled every 400 ms on the PC.
- A page with `"app": "photoshop.exe|lightroom.exe"` is shown while one of those processes is in front; the deck returns to the page it came from when the focus moves to an application without a page. Manual page switches end the auto mode until the next focus change.
- The configurator's "Apps" library creates such pages from presets (hotkey sets per application); any page can get a pattern in Settings, with a picker of currently running applications.

### 3.5 AgentDeck (coding-agent sessions)

- Plugin `agentdeck` mirrors the AgentDeck daemon (local WebSocket, port 9120): one tile per session (roster order, 8 slots) coloured by state, a selected-session tile, option tiles whose labels follow the live prompt, and Yes/No/Stop keys acting on the selected session. Selection: explicit press, else the first waiting session, else the daemon's focused one.
- Observed (hook-only) sessions can only be answered when the daemon holds a permission gate (`requestId`) or marks the prompt `liveAnswerable`; otherwise the tile shows the question and the answer must be typed in the terminal, as in AgentDeck's own decks.

## 4. Home Assistant plugin (phase 2)

- Connection: HA WebSocket API (`ws(s)://host/api/websocket`), long-lived access token (the user creates it in HA; stored in the agent's config, not on the deck). Same mechanism Ulanzi's plugin uses (auth, `get_states`, `subscribe_events state_changed`, `call_service`).
- Ops: `toggle`, `turn_on`, `turn_off`, `call_service` (domain, service, data), `scene`, `script`, `set_value` (number/input_number via knob with step), `light_brightness` (knob: +/- 10 %), `climate_temp` (knob), `media_player` (play/pause/next/prev/volume via knob).
- States: entity `state` string (`on`/`off`/`playing`/...), plus formatted `value` strings from `state` + `unit_of_measurement` and attribute picks (brightness %, temperature). The agent subscribes once and pushes deltas; after reconnect it pushes a full snapshot.
- Icons: HA entities carry `icon` (`mdi:...`) or a default per domain/device_class. The agent renders Material Design Icons (SVG path data from the `@mdi/svg` package, Apache-2.0) to PNG at the tile size in the look's colour, uploads them as `mdi-<name>-<colour>.png`, and records them in the config looks. Default look pairs for toggles: on = coloured icon on the entity's "on" colour, off = grey outline.
- Configurator: "Home Assistant" tab: URL + token, test connection, entity picker with search and domain filter, "add as toggle key / as sensor tile / as knob (brightness, volume, temperature)", automatic looks and icons. Looks remain editable.

## 5. Icons: sources, formats, budget

- On device: PNG (RGBA) at the content-box size (124x110 at `text_pad` 18) stored in `/data/deckd/icons`; decoded on demand, cached in RAM, evicted when memory is short. JPEG for photos (no alpha). Animated GIF: later, with a frame budget (RAM is 18 MB free).
- Storage: the `data` partition is 4 MiB JFFS2 (compressed); configs are a few KB, logs rotate at 60 KB, so roughly 3.5 MiB for icons: about 250 to 500 typical icons. The configurator shows the used budget and offers "remove unused icons".
- Sources in the configurator: (1) any PNG/JPG/SVG file (resized, SVG rendered with `resvg`); (2) Material Design Icons with colour/background options (rendered locally, no network); (3) the Ulanzi icon packs already on this PC under `%APPDATA%\Ulanzi\UlanziDeck\Icons` (local use); (4) auto-generated text tiles (big glyph or two letters) for keys without icons.
- Looks per state give two-state icons (on/off, muted/unmuted, recording/not) without the deck knowing what they mean.

## 6. Phases

1. **Host channel + looks (device + agent core)**: `host` action with fallback, `looks`/`state`/`value`, agent heartbeat, events, results, red flash, status dot, `system` plugin (open/url/close/media keys), tray agent. Testable with no network service.
2. **Home Assistant**: WebSocket client, token config, ops above, state subscription, MDI icon rendering and upload, configurator tab with entity picker. Acceptance: toggle a light from a key with live on/off look, show a temperature on a tile, control a light's brightness with a knob, all with Ulanzi Studio closed; keys of type hotkey keep working with the agent closed.
3. **More plugins** as wanted: OBS (obs-websocket), Spotify, Discord, HTTP request, soundboard. Device niceties: long-press/double-press, random/2-state switch, knob acceleration.

## 7. Risks and limits

- Media transport keys (play/pause/next) cannot come from the deck alone with the stock USB descriptor; they go through the agent (or a future kernel-driver change).
- Only one host program may hold the vendor interface: the agent and Ulanzi Studio are mutually exclusive, and the configurator shares the agent's connection rather than opening its own.
- Deck RAM (18 MB free) bounds icon caching and rules out animated backgrounds everywhere; the firmware must evict decoded icons under pressure.
- Security: the HA token lives on the PC in the agent's config file with user-only permissions; the deck only ever sees entity ids and state strings.
