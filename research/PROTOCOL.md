# D200X vendor HID protocol, as observed

Source: `captures/studio-startup.pcap` (Ulanzi Studio 3.4.4 start-up and a few key presses, 2026-10-29), decoded by `captures/parse_capture.py` into `captures/payloads/`. Deck: D200X rev 02, firmware 5.3.6.

## Transport

- Interface 0 of USB device 2207:0019. Endpoint 1 OUT (host to deck), endpoint 2 IN (deck to host). Both 1024-byte interrupt reports. USBPcap shows a leading 0x00 report-ID byte on Windows writes.
- Frame: `7c 7c | cmd u16 BE | len u32 LE | payload`. A payload longer than 1016 bytes continues in raw 1024-byte reports until `len` is satisfied. The deck's replies always declare `len = 1014` (0x3f6) and pad with NUL.

## Host to deck commands seen

| cmd | name | payload | notes |
|---|---|---|---|
| 0x0003 | GET_DEVICE_INFO | empty | Sent twice at start-up. Reply is 0x0303 |
| 0x000b | SET_LABEL_STYLE | JSON `{"Align":"bottom","Color":16777215,"FontName":"Source Han Sans SC","ShowTitle":true,"Size":10,"Weight":80}` | Sent before the first bundle |
| 0x0001 | SET_BUTTONS | ZIP: `manifest.json` + `Images/*.png|gif` | Full page. 58 KB and 17 KB bundles observed, one per page switch |
| 0x000a | SET_BRIGHTNESS | ASCII `100` | Also used as keep-alive |
| 0x001a | unknown | ASCII `0` | Always right after brightness; likely online-sleep minutes |
| 0x0006 | SET_SMALL_WINDOW | `mode|cpu|ram|HH:MM:SS|gpu|24H|` e.g. `0|18|29|19:57:49|7|24H|` | Every 5 s. Mode 1 seen at page changes |
| 0x000d | UPDATE_KEYS | ZIP with a manifest containing only the changed key(s) and their icons | Per-key partial update exists, contrary to the community plugins' assumption |
| 0x00e0 | SET_KEY_STATE | ZIP with `manifest.json` = `{"4_0_0":{"KeyType":1,"Progress":100,"State":1}}` then `{"4_0_1":...}` | Sent twice after each key press (state 0 then 1): visual press feedback |

## Deck to host

| cmd | payload | notes |
|---|---|---|
| 0x0303 | JSON `{"SerialNumber":"02D23A045U3672338","Dversion":"5.3.6","error":"0","DeviceType":"D200","HardwareVersion":"SSD210V100"}` | Reply to 0x0003. Note `DeviceType` says D200 even on a D200X; the host decides the model |
| 0x0101 | `01 04 01 01 00...` press, `01 04 01 00 00...` release | Observed for the key at manifest slot `4_0` (top right). Byte layout guess: `[1][col=4][row+1 or type=1][pressed]`. Needs more samples from `d200x-tool monitor` (dials are reported at indices 17-19 per the OpenDeck plugin) |

## Manifest format (SET_BUTTONS / UPDATE_KEYS)

```json
{
  "0_0": {"State": 0, "ViewParam": [{"Font": {...label style...}, "Icon": "Images/<uuid>.png", "Text": "Tisch"}]},
  "1_3": {"State": 0, "ViewParam": [{"Font": "", "Text": ""}]},
  "3_2": {"SmallViewMode": 0, "State": 0, "ViewParam": [{"Font": {...}, "Text": ""}]},
  "4_3": {...}
}
```

- Keys are `"<col>_<row>"`, columns 0..4, rows 0..3. Rows 0..2 are the 5x3 LCD grid. Row 3 (`0_3`, `1_3`, `2_3`, `3_3`, `4_3`) is present in every manifest and matches the knob/side-button row: the three dials and two side buttons get entries too.
- `"3_2"` is the wide slot (spans `3_2` and `4_2`; `4_2` is absent) with `SmallViewMode` 0 (stats), 1 (clock), 2 (image).
- Icons are PNG or animated GIF. The GIF observed was 21 KB.
- An empty key is `{"Font": "", "Text": ""}`.
- `State` on a key and `KeyType`/`Progress`/`State` in 0x00e0 drive the on-device highlight; `KeyType` 1 with `Progress` 100 is a plain momentary key.

## On-device input map (from the deck app's log, `adb-keylog/20261029_202558/`)

The MCU on `/dev/ttyS1` delivers two values per event, `matrixKey` (bitmask) and `discreteKey`. The app numbers inputs as follows; these are the same indices the HID input reports and the OpenDeck plugins use.

| index | input | matrixKey bit | manifest slot |
|---|---|---|---|
| 0..12 | LCD keys, reading order, row by row | `0x4000 >> index` (key 0 = 0x4000, key 12 = 0x0004) | `col_row` with col = index % 5, row = index / 5 |
| 13 | wide key ("small window") | 0x0001 | `3_2` |
| 15, 16 | side buttons (page keys) | discreteKey | `0_3`, `1_3`? (row 3 entries) |
| 17, 18, 19 | knob 1..3 press | discreteKey | `2_3`, `3_3`, `4_3` |
| knobs | rotation logged as `KNOB_n left/right` | discreteKey | same |

Observed in the log: keys 0..13 press/release, side buttons 15 and 16, knob presses 17..19, rotations for knobs 1 and 2 (knob 3 was not turned during the test).

## MCU serial protocol (`/dev/ttyS1`, 115200 8N1, raw dump `adb-uartdump/20261029_203909/`)

Frame: `FF 55 | cmd u16 LE | len u8 | payload[len] | sum u16 BE`, where sum is the 16-bit sum of all preceding bytes including `FF 55`. 1547 frames, zero checksum errors. Nothing was received at 1500000 baud.

| cmd | len | payload | meaning |
|---|---|---|---|
| 0x0001 | 3 | `D M1 M0` | Input state, sent on every change (press and release). `M1:M0` big-endian = the app's `matrixKey`: key n (0..12) = `0x4000 >> n`, wide key = `0x0001`. `D` = discrete bits: 0x01 side button 1 (index 15), 0x02 side button 2 (16), 0x04 knob 1 press (17), 0x08 knob 2 press (18), 0x10 knob 3 press (19). Observed combos like `12 00 00` are overlapping presses |
| 0x0002 | 1 | nibble | Knob 1 rotation step |
| 0x0003 | 1 | nibble | Knob 2 rotation step |
| 0x0004 | 1 | nibble | Knob 3 rotation step |

Rotation nibble = `(new_AB << 2) | old_AB` of the encoder's quadrature pins, one frame per state transition, four per detent. Only two sequences occur: `01 -> 07 -> 0e -> 08` (one direction) and `0d -> 04 -> 02 -> 0b` (the other). So direction is readable from a single frame: set {01,07,0e,08} vs set {0d,04,02,0b}. Which set is clockwise still has to be tied to the app's `KNOB_n right/left` log with one combined test.

Host-to-MCU queries, recovered by disassembling the vendor app (`research/disasm.py`, `KeyProto::keyProtoParse`):

| frame | vendor name | reply |
|---|---|---|
| `FF 55 01 00 00 01 55` | `queryKeyStatus` | cmd 0x0001 state frame |
| `FF 55 10 00 00 01 64` | `queryMcuVersion` | cmd 0x0010, ASCII version |
| `FF 55 FE 00 04 00 01 02 03 ..` | MCU update handshake (`McuUpdate`) | do not send |

The earlier `FF 55 11 00 01 65` probe was simply malformed and ignored. deckd sends the two queries at start-up.

## Serial number

Not in any flash partition in plain text. The stock app's `DeviceInfo::firmwareSN` calls `SecurityManager::readSecData(buf, 17, page 3)` in `libeasyui.so`, whose OTP-flash backend does `open("/dev/oflash", O_RDWR)`, optionally `ioctl(fd, 0x400f320f, <15 bytes>)`, then `ioctl(fd, 0x80086f04, &{page u8, len u8, pad u16, buf ptr u32})`. Page 3 holds the 17-character serial; it is the SPI NOR's OTP security register. The strings in the library are obfuscated, so the path is inferred from its length (11) and the existing `/dev/oflash` node. Implemented in `deckd/src/serial.rs`. On the deck (2026-10-30): the authentication ioctl `0x40046f03` with token `0x1d2f2736` succeeds, but the read ioctl still returns EPERM, so the driver gates reads on more of the vendor's (obfuscated, anti-tamper) authorisation sequence. Not pursued further: the install and dry-run scripts capture the serial from the stock firmware's own 0x0303 reply and store it in `/data/deckd/serial`, which deckd uses for the USB descriptor and its info reply. MCU firmware version on this unit: `V0.0.05`.

## Display (verified with `d200x-device/deckproto`, run `adb-deckproto/20261029_204358/`)

`/dev/fb0` (sstarfb): 540x960 physical, virtual 540x1920 (two pages), 32 bpp ARGB8888 (r@16 g@8 b@0 a@24), stride 2160, smem 4 MiB, yoffset 0. Writing page 0 and issuing FBIOPAN_DISPLAY shows immediately; the vendor display daemon `zkdisplay` keeps scanning out while the app is dead. Mapping a 960x540 landscape canvas: physical `(px, py) = (ly, 959 - lx)` (rotation 3 in deckproto). Rotation 1 produced a 180-degree flipped image.

Live decode in the same run matched the physical keys (user-confirmed), side buttons gave discrete bits 0x01/0x02, knob presses 0x04/0x08/0x10, knob rotations decoded as +/-1 steps with no checksum errors.

## USB gadget (device side)

Switching: `setprop sys.usb.config hid` (init rewrites `/sys/class/zkswe_usb/zkswe0/{idVendor,idProduct,functions,enable}`, see `init.rc`); `adb` switches back. The `/dev/hidg*` nodes exist only in `hid` mode:

| node | USB interface | report |
|---|---|---|
| `/dev/hidg0` | interface 0, usage page 0x0C, 1024-byte IN/OUT, no report ID | vendor channel (the 7c7c protocol) |
| `/dev/hidg1` | interface 1, boot keyboard | 8 bytes: modifiers, reserved, 6 keycodes, no report ID. 1-byte LED output report |

Host-side descriptors (via `d200x-tool descriptors`):
- if1: `05 01 09 06 a1 01 05 07 19 e0 29 e7 15 00 25 01 75 01 95 08 81 02 75 08 95 01 81 03 19 00 29 65 15 00 25 65 75 08 95 06 81 00 05 08 19 01 29 05 15 00 25 01 75 01 95 05 91 02 75 03 95 01 91 03 c0`
- if0: `05 0c 09 01 a1 01 09 01 15 00 26 ff 00 75 10 96 00 02 81 02 09 01 15 00 26 ff 00 75 10 96 00 02 91 02 c0`

Verified 2026-10-29 with `d200x-device/hidkeys`: writing boot reports to `/dev/hidg1` typed "abcdefghijklm" and Enter into Notepad with no host software running. Writes to `/dev/hidg0` are accepted but go to the vendor interface. No consumer-control (media key) usage exists in the stock keyboard descriptor; volume keys would need a changed descriptor, which lives in the vendor gadget driver, not in user space.

## Stock standalone mode

With no host attached (`mode=offline`), the app loads `/res/ui/default/manifest<N>.json` (N selected by `DefKeyMode`, 2 = macOS set) and executes key actions itself through the keyboard gadget `/dev/hidg0`. Actions are `com.ulanzi.ulanzideck.system.open` with a `Path` keyword from a fixed table compiled into the app:

`calc wordpad cmd notepad control perfmon devmgmt.msc powershell explorer taskmgr pscreenshot alttab volumeSw maccopy macsave macpaste macsearch macmultitask maccut macscreenshot maccancel macpscreenshot macredo macctla safari.app music.app mail.app keynote.app calendar.app "app store.app" notes.app numbers.app finder.app pages.app`

Knobs in standalone mode do `volume++`, `volume--` and `volume switch`. There is no generic "send these HID keycodes" action, so the stock firmware cannot run user-defined shortcuts offline. A configurator that only speaks this protocol cannot deliver true standalone operation; the on-device app has to be replaced.

## Timing seen

- Start-up: GET_DEVICE_INFO, SET_LABEL_STYLE, GET_DEVICE_INFO, SET_BUTTONS (58 KB, 0.46 s to send), brightness, 0x001a, brightness, 0x001a.
- Keep-alive: SET_SMALL_WINDOW every 5 s, brightness roughly every 3 s.
- Each key press: deck sends 0x0101 press, host answers with two 0x00e0 state ZIPs within 5 ms, deck sends 0x0101 release 60-300 ms later.
