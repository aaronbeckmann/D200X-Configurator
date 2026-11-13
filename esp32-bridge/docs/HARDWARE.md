# D200X teardown: what the bridge mod relies on

Source: `docs/disassembly/IMG_1919..1924.HEIC` (2026-11-02), continuity tracing, a scope, and the ADB dumps. Wiring for the mod: `WIRING.md`.

## Two boards, one cable

The D200X is a D200 deck module sitting on top of a USB-C dock board. The only link between them is a short USB cable.

| Board | Silk | Main parts |
|---|---|---|
| Dock (bottom) | `HBC125 V1.3 20250912` | Genesys **GL3590** (USB 3.2 hub, 05E3:0610/0625), Genesys **GL850S** (USB 2.0 hub, 05E3:0608, 12 MHz crystal), Genesys **GL3231** (SD/microSD card reader, 25 MHz crystal), GScoolink **GSV2201S** next to HDMI (USB-C DP alt mode to HDMI, plus the billboard + CDC device 3679:0001), **GPD81048** by the 3.5 mm jack, buck converters. Ports: 3x USB-C, 2x USB-A, HDMI, SD, microSD, 3.5 mm jack |
| Deck (top) | `Ulanzi_D200_MB (A01/A044/A045) Ver:V1.0`, SN label `02D23A045U3672338` | SigmaStar SSD210, Puya SPI NOR (32 MiB), Puya **PY32F031K18** (key/encoder MCU on ttyS1, firmware `V0.0.05`), unpopulated 5-pin header (the MCU's SWD port), test pads `TP3`, buck converter, 220 µF 25 V input cap, red connector `CN2` (silk `USB1`) |

## The dock-to-deck cable

- Dock side: white 6-pin connector right above the GL850S. Deck side: `CN2`.
- Wires: 5 V, D+, D-, GND, GND, and a green wire that stays at 0 V with no activity at all (scope, including a replug): a static enable/reset/ID line or a ground. The mod leaves the whole cable untouched, so keystrokes keep their path to the PC.

## Dock USB topology (Windows, deck connected, SD + microSD inserted, display on HDMI)

```
05E3:0610 (GL3590 USB 2.0 half)
  port 4 -> 05E3:0608 (GL850S)
              port 1 -> 2207:0019 deck (serial 02D23A045U3672338)
              port 2 -> 3679:0001 GSV2201S billboard + CDC
              ports 3, 4: nothing (traced to the GL3231's USB 2.0 pair; idle on a USB 3 host)
05E3:0620 -> 05E3:0625 (GL3590 SuperSpeed half)
  port 4 -> 05E3:0764 card reader (GL3231), two LUNs: SD and microSD
```

The GPD81048 by the jack does not enumerate as a USB device; it is probably fed analog or I2S audio from the GSV2201S.

## Deck console UART (where the ESP32 sits)

From the ADB dumps (`research/adb-backup/20261029_200910/`):

- `console=ttyS0,115200`, `loglevel=0` (the kernel is almost silent on the console).
- Only `ttyS0` (console) and `ttyS1` (key MCU) exist.
- `init.rc` starts `service console /bin/sh` on the console (`ro.debuggable=1`). So **ttyS0 carries a root shell** whenever deckd has not stopped it. The bridge firmware keeps its TX in high impedance until it hears a valid deckd beacon, and deckd kills that shell before it opens the link (`INTERFACE.md` section 4).

Found with `research/adb-uarttest.ps1` (pattern `UUUU... ttyS0 test <n>` at 115200) and a scope:

- **The pin next to `TP3`** carries the pattern: **SoC console TX** (deck -> ESP32 RX).
- **The far pin** of the pair is steady at **3.3 V**: **SoC console RX** with its pull-up (ESP32 TX -> deck).
- TX edges are slow: rise time **1.85 µs** (10x probe; 7 µs with the first probe setup). One bit is 8.68 µs at 115200, so 115200 works with margin; 230400 is borderline; 921600 is impossible as is. For faster rates put a Schmitt buffer (74LVC1G17, 3.3 V) right at the tap and run the wire from its output. Keep the deck-TX wire short.
- Levels are 3.3 V on both sides: direct connection to the ESP32.

## The 5-pin header

Its vias go to the PY32F031K18, not to the SoC: the MCU's programming port, almost certainly 3V3 / SWDIO (PA13) / SWCLK (PA14) / NRST / GND. Not used by the mod. Never write to it: the MCU drives all keys and knobs, and the stock app's `updateMcu` is the only known recovery.

## Power

The bridge takes 5 V from `CN2` (USB VBUS from the dock) through a Schottky diode, and makes its own 3.3 V. The deck's 3.3 V rail is never loaded: Wi-Fi peaks (up to about 500 mA) could pull it down and reset the SoC. The 25 V input cap on the deck suggests the deck tolerates more than 5 V; measure VBUS if the dock is powered from a PD charger.
