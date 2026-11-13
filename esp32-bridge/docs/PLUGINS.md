# Bridge plugins defined in the config (no firmware update)

The ESP32 bridge (`esp32-bridge/`) runs built-in plugins (`homeassistant`, `mqtt`, `net`, `obs`, `spotify`) and any number of plugins described in its config under `"plugins"`. A described plugin behaves like a built-in one: it appears in the bridge's heartbeat, deck keys call it with `{"type":"host","plugin":"<name>","op":"<op>"}`, and its states (`<name>:<key>`) feed tiles, looks and rings. Adding or changing one is a config change (`d200x-tool bridge-config bridge.json`); the bridge restarts in about two seconds.

## Shape

```json
"plugins": {
  "<name>": {
    "conn": "<connection>",                  // base URL + headers from "connections", or:
    "url": "http://192.168.1.50",            // base URL
    "headers": { "X-Api-Key": "..." },       // sent with every request
    "ops":    { "<op>": <template>, ... },
    "states": { "<key>": <state source>, ... },
    "refresh": true                          // re-poll the states right after an op (default)
  }
}
```

Names: lowercase letters, digits, `_`, `-`. Not allowed: the built-in names and those of PC-only plugins (`homeassistant`, `ha`, `mqtt`, `net`, `obs`, `spotify`, `system`, `media`, `discord`, `agentdeck`, `hid`).

## Op templates

| Form | Meaning |
|---|---|
| `{"method": "POST", "path": "/x", "headers": {..}, "body": {..} \| "text", "content_type": ".."}` | HTTP (the default `"type"`). `path` is appended to the base URL; `"url"` replaces it. Without `method`: POST with a body, GET without. Success = HTTP 2xx |
| `{"type": "mqtt", "topic": "..", "payload": "..", "retain": false, "qos": 0}` | MQTT publish (needs the `mqtt` section) |
| `{"type": "udp", "host": "..", "port": 4210, "payload": ".." \| "hex": "01 ff"}` | one UDP datagram |
| `{"type": "wol", "mac": "aa:bb:cc:dd:ee:ff", "broadcast": "192.168.1.255", "port": 9}` | Wake-on-LAN |
| `[ <template>, <template>, ... ]` | run in order, stop at the first failure |
| `{"press": <template>, "turn": <template>}` | by event: key/knob press, knob turn (an event without a template does nothing) |

## Expressions

Any string in a template may contain `${...}`. A string that is exactly one `${...}` keeps the value's JSON type (number, bool, array); otherwise the value is inserted as text.

| In `${...}` | Value |
|---|---|
| `delta` | knob detents of this turn (several quick detents arrive summed), 0 on press |
| `event` | `press` or `turn` |
| `args.<path>` | the key's `args`, e.g. `args.id`, `args.rgb.0` |
| `state:<key>` | this plugin's state `<name>:<key>` |
| `state:<prefix>:<name>` | any state the bridge knows, e.g. `state:ha:light.desk` |
| `+ - * / ( )`, numbers | arithmetic (state text such as `"128"` counts as a number) |
| `min(a,b)` `max(a,b)` `clamp(x,lo,hi)` `round(x)` | numeric helpers |
| `not(x)` `bool(x)` | JSON booleans; true for non-zero numbers and `true`/`on`/`yes`/`open` |

## State sources

| Form | Meaning |
|---|---|
| `{"path": "/x" \| "url": "..", "pointer": "/a/0/b", "every_s": 5}` | HTTP GET (or any template fields) every `every_s` seconds (default 30); the value at the JSON pointer, or the first line of the body |
| `{"topic": "home/+/door", "pointer": "/state"}` | MQTT subscription (needs the `mqtt` section); the payload, or a field of a JSON payload |
| `"map": {"true": "on", "false": "off"}` | optional, in either form: rewrite values |

## Examples

WLED light strip (HTTP JSON API), with a knob for brightness:

```json
"wled": {
  "url": "http://192.168.1.50",
  "ops": {
    "toggle": { "method": "POST", "path": "/json/state", "body": { "on": "t" } },
    "bri": { "press": { "method": "POST", "path": "/json/state", "body": { "on": "t" } },
             "turn":  { "method": "POST", "path": "/json/state",
                        "body": { "bri": "${clamp(state:bri + delta * 16, 1, 255)}" } } },
    "preset": { "method": "POST", "path": "/json/state", "body": { "ps": "${args.id}" } }
  },
  "states": {
    "on":  { "path": "/json/state", "pointer": "/on", "every_s": 5, "map": { "true": "on", "false": "off" } },
    "bri": { "path": "/json/state", "pointer": "/bri", "every_s": 5 }
  }
}
```
Deck: key `{"type":"host","plugin":"wled","op":"toggle"}` with `state: "wled:on"` and on/off looks; knob `press`/`left`/`right` = op `bri`; a ring with `state: "wled:bri"`, max 255.

Shelly Plug (Gen2 RPC):

```json
"shelly": {
  "url": "http://192.168.1.60",
  "ops": { "toggle": { "path": "/rpc/Switch.Toggle?id=0" } },
  "states": { "on":    { "path": "/rpc/Switch.GetStatus?id=0", "pointer": "/output", "every_s": 3, "map": { "true": "on", "false": "off" } },
              "power": { "path": "/rpc/Switch.GetStatus?id=0", "pointer": "/apower", "every_s": 3 } }
}
```

Tasmota over MQTT:

```json
"desklamp": {
  "ops": { "toggle": { "type": "mqtt", "topic": "cmnd/desklamp/POWER", "payload": "TOGGLE" } },
  "states": { "power": { "topic": "stat/desklamp/POWER" } }
}
```

Node-RED or any webhook, with a token kept on the bridge:

```json
"connections": { "nodered": { "url": "http://192.168.1.2:1880", "headers": { "Authorization": "Bearer ..." } } },
"plugins": { "flows": { "conn": "nodered",
  "ops": { "run": { "method": "POST", "path": "/deck/${args.flow}", "body": { "key": "${args.flow}" } } } } }
```

## Limits

- Templates cover request/response and publish/subscribe services. Services that need a login handshake, a session or a binary protocol need code: either a new built-in plugin (firmware update over Wi-Fi, see `WIRING.md` section 7) or, later, the script engine planned for the `storage` partition.
- Expressions have no conditionals or string functions beyond `not`/`bool`; two ops selected by the key (for example `on` and `off` keys, or looks) cover most cases.
- One poll task serves all described plugins; a slow endpoint delays the others' polls (each request times out after 8 s).
