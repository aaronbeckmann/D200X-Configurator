# Ulanzi D200X custom firmware investigation

Date: 2026-10-29. Device under test: the D200X on Aaron's desk, serial `02D23A045U3672338`, stock firmware 5.3.6, currently enumerated on this PC.

## 1. Verdict

Custom firmware that makes the D200X work standalone, configured by a Rust tool that is then unplugged, is feasible. The big surprise is which chip you have to target.

- The D200X on the desk is **not** the Rockchip RK3308 board that the D200 and the existing open-source firmware (riptidewave93/d200-buildroot) use.
- It is hardware revision 02, built on a **SigmaStar SSD210** (dual Cortex-A7, 64 MB in-package DDR2, SPI flash), running SigmaStar's vendor Linux 4.9 plus the "ZK EasyUI" application framework. The same vendor stack sits in Ulanzi's TC002 pixel clock, which already has a community customisation project.
- Both revisions speak the identical USB HID protocol, so host-side work (a Rust configurator) is portable across revisions, but on-device work is not.
- The stock firmware's update container (`ZKSWEV1.0`) is **unsigned** and only checks CRC-32, MD5 and a device code. The app lives in its own 8 MiB `res` partition. Replacing just that partition with your own program, while keeping the vendor kernel, bootloader and recovery, is the realistic path. It is the same approach the TC002 project took.
- Recommended order of work: (1) non-destructive discovery with ADB, Wireshark and the existing HID protocol; (2) a Rust host tool that drives the stock firmware; (3) a replacement on-device app flashed through the vendor update path; (4) only if needed, deeper bootloader/kernel work.

## 2. What the device actually is

### 2.1 USB enumeration on this PC

The deck shows up behind its own internal hub tree. The relevant devices (from `Get-PnpDevice`):

| Device | USB ID | Role |
|---|---|---|
| "ulanzi" composite | 2207:0019 | The deck SoC. Interface 0 = vendor HID (1024-byte reports, usage page 0xFFxx), interface 1 = boot keyboard |
| GsCooLink | 3679:0001 rev 0113 | USB Billboard + CDC serial (COM3). Dock / USB-C alt-mode controller, independent of the SoC |
| Genesys hub | 05E3:0608 | USB 2.0 hub that carries the two devices above |
| Genesys hubs | 05E3:0610 / 05E3:0625 | USB 2.1 / 3.2 halves of the dock's main hub |
| "ulanzi" ADB | 18d1:d002 | Seen once, 2026-10-27 16:33:18 to 16:33:23, bound to WinUSB as "ADB Device". Not present now |

Vendor ID 2207 is Rockchip's, which is why everyone assumed RK3308. Ulanzi simply kept the same VID:PID on the SigmaStar revision so the desktop software does not care.

### 2.2 Evidence that revision 02 is SigmaStar SSD210

1. Ulanzi Studio 3.4.4 ships two on-device payloads for the D200X in `C:\Program Files (x86)\Ulanzi Studio\Ulanzi\UlanziDeck\defProfile\D200X\bin\`:
   - `01/bin.zip`: `UlanziDeckKey`, a 2.1 MB 32-bit ARM (armhf) Qt5 ELF with paths like `/userdata/UlanziDeckKey`, `/dev/hidg0`, `/dev/hidg1`. This matches what the Hackaday/badustudio research found on the RK3308 D200 (Buildroot userland, ARMv7).
   - `02/01/bin.zip`: `update.img` (3.3 MB), `binversion` = `5.3.6`, `md5.txt`. The image is a `ZKSWEV1.0-180127` container wrapping one squashfs for mtd index 3 (`res`), device code `0xaa550606`, device flag 0 (SPI NOR). Identical container format and device code to the TC002 (SSD210).
2. Inside that squashfs: `lib/libzkgui.so` (the deck app, linked against `libeasyui.so`, `libzkhardware.so`, `libzknet.so`), `etc/EasyUI.cfg`, `ui/`, fonts, `bin/minizip`, `bin/logkeep`. The library contains the strings `SSD210V100`, `SSD210V101`, `SSD210V102` (hardware version table), `/sys/devices/soc0/soc/soc:usbotg/otg_role`, `/sys/class/zkswe_usb/zkswe0/iSerial`, `sys.zkupgrade.flag`, `/proc/mtd`.
3. An independent user report (AgentDeck issue 174) shows a D200X on firmware 5.3.6 reporting hardware ID `SSD210V100`.
4. Serial prefix: the RK3308H3 recovery guide says serials starting `01` are the Rockchip H3 board. This unit starts with `02`, and the 02 payload directory is the SigmaStar one.

Full header decode and extracted file listing: `research/stock-firmware/`.

### 2.3 Stock firmware architecture (revision 02)

Pieced together from `libzkgui.so`, `EasyUI.cfg`, and the TC002 project's documentation of the same stack:

- Boot chain: SigmaStar BootROM -> IPL -> IPL_CUST -> vendor U-Boot 2015.01 -> Linux 4.9.84 (SigmaStar SDK, "Ikayaki"/Pioneer3) -> `init` -> `zkswe` loader -> `libzkgui.so` via `startupLibPath`.
- Flash: SPI NOR (device flag 0), eight MTD partitions on the TC002 (`BOOT0`, `KERNEL`, `rootfs` squashfs, `res` squashfs, `config`, `MISC`, `data` jffs2, `UDISK` vfat holding a factory `update.img`). The D200X layout is unverified but the app reads `/proc/mtd` and `/dev/block/mtdblockN`, and the update targets index 3.
- Display: `wallpaper.jpg` is 1280x720, `rotateScreen: 90`, so the panel is a 720x1280 portrait TTL/RGB panel used in landscape, exactly like the D200. Keys are windows onto one panel, not separate displays.
- Input: `touchDev: /dev/input/event0` (actually the key matrix, no touch). Knobs: `KNOB_1..3 left/right`, `knobRotate`, `parseKnobLayouts`, `SET_SMALLWINDOW_TO_KNOB`. The encoders sit behind a serial link: `uart: ttyS1`, `baud 115200`, `SerialTransport`, `EncoderVersion`, and `update Mcu start/success/failed`. So there is a **separate MCU** for the encoders (and probably the side buttons), with its own updatable firmware, as on the TC002.
- USB gadget: `/dev/hidg0` (keyboard, used in standalone mode) and `/dev/hidg1` (vendor HID channel), `switch usb_device`, `usbMode`. ADB exists but is only exposed briefly (observed once for 5 s, most likely during a reboot right after Ulanzi Studio first connected and pushed an update).
- Standalone mode already exists in stock firmware: `send standalone start`, `DefKeyMode`, `KeyMode`, offline sleep timers, `/tmp/standalone/` and `:/resource/standalone/default.zip` in the RK build. The deck can run keyboard-shortcut keys by itself after the host goes away. What is unknown is how far that goes (pages, knobs, icons) and how Ulanzi Studio provisions it.
- Protocol IDs seen in the app: `SETKEYPAD`, `SETSCREEN_PIC`, `DRAW_JS_IMG`, `SET_SMALLWINDOW_TO_KNOB`, `LOCKSCREEN`, `UNLOCKSCREEN`, `RUN_RESULT`, `UPDATE_BIN`, `APP_EXIT`, `SHUTDOWN`, plus version comparison `Firmware version:%s < Host version:%s` (the host decides whether to push `update.img`).

### 2.4 Wire protocol (already reverse-engineered by the community)

1024-byte HID reports on interface 0: header `7c 7c`, u16 big-endian command, u32 little-endian length, payload. Known commands: `0x0001` SET_BUTTONS (a ZIP with `manifest.json` plus icons, sent in 1024-byte chunks), `0x0006` small window, `0x000a` brightness (ASCII 0-100), `0x000b` label style, `0x0101/0x0102` input events. Hardware indices: keys 0-13, side buttons 15-16, dials 17-19 (dial byte: 0 release, 1 press, 2 left, 3 right). Icons are 196x196. Firmware bug: a ZIP byte of `0x00` or `0x7c` at offset `1016 + 1024*N` corrupts the upload, so bundles are padded. Only whole-bundle updates, no per-key images. Host silence for about a minute drops the deck back to its boot logo.

Sources: kastro5/opendeck-ulanzi-d200, jcalado's Companion port, strmdck. Windows needs a leading report-ID byte `0x00` on writes.

## 3. The open-source landscape

| Project | Target | Relevance |
|---|---|---|
| riptidewave93/d200-buildroot + d200-deck + d200-configure | D200 (RK3308B-S, SPI NAND, NV3052C panel). Mainline U-Boot 2026.10 + Linux 7.2.11, Go key app, A/B OTA, WebHID configurator, standalone keyboard/macro keys, deckhost PC helper | The reference design for exactly what you want, but for the other silicon. Its config format, HID config channel (usage page 0xFFD2, seq/cmd framing, config-read/write/commit), and standalone-first philosophy are worth copying. Only useful on-device if you ever get a revision-01 D200X |
| atomicstack/tc002-customisation | Ulanzi TC002 (SSD210, same ZK stack) | Documents the ZKSWEV1.0 container byte layout, the `zk_upgrade_*` flasher, `zkdaemon` recovery, ADB root over USB with `otg_role`, and ships a replacement runtime in Zig cross-compiled to static armv7. This is the closest blueprint for revision 02 |
| kastro5 / edubox / LuizGustavoWT OpenDeck plugins, bitfocus Companion surface, strmdck | Host side | Full HID protocol including dials and side buttons. Any Rust host tool can start from these |
| linux-chenxing (Pioneer3 page), youheng7185/linux_sigmastar_ssd210 | SSD210 mainline | Pinout, bootstrap pins, SPI NAND layout, JTAG. No mainline kernel boots on Pioneer3 yet beyond a serial console on a hobby board. Vendor Linux 4.9 is the only working kernel |
| SigmaStar docs (comake mirrors) | SSD2xx | `USBDownloadTool` blank-flash recovery: BootROM enters USB download when no IPL is found, and the PC preloads a U-Boot with the "ufu" function. Also strap pin `PM_SPI_HLD` for USB boot |
| Misyuk-T and CHADREX recovery repos | D200/D200H RK3308 | Rockchip RKDevTool recovery, pad locations. Not applicable to revision 02 |

## 4. Paths to a standalone deck with a Rust configurator

### Path A: no firmware change, drive the stock standalone mode (lowest risk, start here)

The stock app has a standalone key mode and a keyboard gadget. If Ulanzi Studio can provision shortcut keys that keep working after disconnect, then a Rust tool that speaks the existing 7c7c protocol gives you most of the goal with zero flashing risk.

- Capture Ulanzi Studio with Wireshark + USBPcap (both installed) while you: assign keyboard shortcuts, change pages, set the small window, assign knob actions, then unplug. Diff the manifests. Look for the standalone trigger and `DefKeyMode`.
- Write the Rust host library against the HID channel (`hidapi` or `nusb` crate), reusing the OpenDeck plugins as the spec.
- Limits: anything that needs the PC (launching apps, HTTP) will not run offline; the boot-logo fallback after host silence must be understood; knob behaviour offline is unknown.

### Path B: replace the application partition (`res`) only (recommended end state)

Keep vendor BootROM/IPL/U-Boot/kernel/rootfs and the vendor `zkswe` loader and `zkdaemon`; ship your own `res` squashfs whose `EasyUI.cfg` points `startupLibPath` at your program, or whose library is a thin shim that `exec`s a static Rust binary. This is how the TC002 project did it.

- Delivery: build a `ZKSWEV1.0` container (header is fully documented, unsigned) and push it either over ADB (`/mnt/storage` or the `sys.zkupgrade.dir` property) or through the stock app's own `UPDATE_BIN` HID command, which is what Ulanzi Studio uses to push `update.img`. That second route means **no shell access is needed at all** if the command's framing can be captured.
- Recovery: `zkdaemon` reflashes the factory `update.img` from `UDISK` when the reset key is held, and reverts to a stock `EasyUI.cfg` if the app does not report `sys.zkapp.state=running` within 15 s. Your app must set that property. Keep a copy of the stock `res` (you already have the 5.3.6 image) and dump every MTD partition over ADB before writing anything.
- Device app in Rust: `armv7-unknown-linux-musleabihf`, static. Needs: framebuffer or SigmaStar MI display output (open question: does the vendor kernel expose `/dev/fb0`? The RK build used fb0; the ZK stack uses `libzkhardware`), `/dev/input/event0` matrix keys, the ttyS1 encoder MCU protocol (must be sniffed on the UART or lifted from `libzkhardware.so`/`libeasyui.so`, which are in the `rootfs`, not in `res`), `/dev/hidg0` keyboard reports, `/dev/hidg1` or a new usage-page-0xFFD2 gadget for the config channel, and a writable `/data` for the config.
- Configurator in Rust: a CLI first, GUI later. Config model: copy d200-deck's `config.json` (pages, keys, icons, fonts, `hid-key`, `hid-consumer`, `macro`, `page`, with size limits). Transport: a simple seq/cmd/len framing over the vendor HID interface like d200-deck's, with config-info/read/write/commit and an on-device confirmation for new action types.
- Standalone semantics come for free: the device owns the config and executes HID actions itself; the host only pushes config and optionally feeds state (clock, stats).

### Path C: full custom Linux (U-Boot + kernel) on the SSD210 (not recommended now)

No mainline kernel boots on Pioneer3 beyond UART. The vendor SDK kernel is obtainable but closed-ish, and the display, USB gadget, and SPI drivers are all vendor-specific. Recovery from a bad IPL/U-Boot needs SigmaStar's Windows `USBDownloadTool` plus a ufu-enabled U-Boot, or a SPI programmer. This path adds months for no standalone benefit over Path B.

### Path D: get a revision-01 D200X and use d200-buildroot

Everything already exists (mainline Linux, Go app, web configurator, A/B OTA, maskrom recovery over USB with `rockusb`, which has a Rust crate from Collabora). The D200X-specific gaps there are the dials and side buttons. If you are willing to swap hardware, this is the fastest route to a fully open device, and the Rust configurator could target d200-deck's documented config channel directly.

## 5. Risks and unknowns

- **Bricking without a known recovery path.** On revision 02 the only confirmed recovery is the reset-key reflash from `UDISK` (handled by vendor software you would keep) and SigmaStar USB download for blank flash. UART pads are not located on the D200X. Stay within Path B's `res`-only replacement until the UART is found and a full MTD backup exists.
- **Encoder MCU.** Separate firmware on ttyS1 with an unknown protocol. Read-only sniffing first; never run `update Mcu` from a custom app.
- **ADB availability.** It appeared once for 5 s. It may be enabled only during boot or by a host command. Use the boot window, or find the enabling command in the Wireshark capture. On the TC002 the gadget role is switched via `otg_role`.
- **Display path.** Unknown whether a plain `/dev/fb0` exists on the SigmaStar kernel. If the display is only reachable through SigmaStar MI libraries in the rootfs, the Rust app would have to dlopen them or you keep `libzkhardware.so` as a shim.
- **The dock keeps working regardless.** GsCooLink and the Genesys hubs are separate silicon; killing the SoC does not kill the USB-C dock, HDMI, or card readers.
- **Firmware updates from Ulanzi.** Ulanzi Studio compares versions and pushes `update.img` automatically. After customising, never run Ulanzi Studio with the deck attached, or it will overwrite `res`.

## 6. Recommended next steps (all non-destructive)

1. **Capture Ulanzi Studio traffic.** Start USBPcap on the hub that carries the deck, launch Ulanzi Studio, change a key to a keyboard shortcut, set knob actions, change brightness, quit Studio, press keys with Studio closed. Save the pcap into `research/captures/`. This yields: the standalone trigger, `UPDATE_BIN` framing, knob layout JSON, and whether ADB is enabled by command.
2. **Catch the ADB window.** Unplug and replug the deck while running a PowerShell loop over `Get-PnpDevice` for `VID_18D1`, or watch `dmesg` under WSL with usbipd. If ADB appears at boot, run `adb shell` immediately and dump: `/proc/mtd`, `/proc/cmdline`, `dmesg`, `ls /dev`, `/sys/class/graphics`, `cat /sys/devices/soc0/soc/soc:usbotg/otg_role`, `/customer`, `/res/etc/EasyUI.cfg`, `/data`, then `dd` every `/dev/mtdblockN` to the PC. Install `adb` first (`winget install Google.PlatformTools`), since nothing is on PATH right now (the Z: drive copy is unmounted).
3. **Lift the rootfs libraries.** `libeasyui.so`, `libzkhardware.so`, `libzknet.so` are in the rootfs, not in `res`. From the ADB dump, extract them and recover the ttyS1 encoder protocol and the display calls.
4. **Build the Rust host library** against the documented HID protocol and verify it with a bundle push and brightness change. This is useful on both paths and on both hardware revisions.
5. **Prototype a `res` replacement** only after steps 2 and 3: a static Rust binary that draws a test pattern and reports `sys.zkapp.state=running`; package it with the documented ZKSWEV1.0 header; test the reset-key recovery before relying on it.
6. **Find the UART.** Open the case, photograph the board (also settles the flash part number and RAM), locate `GND RX TX` pads near the SoC (TC002 precedent), 115200 8N1.

## 7. Rust building blocks

- Host: `hidapi` (simple, works on Windows/Linux/macOS) or `nusb` (pure Rust, async); `serde`/`serde_json`; `image` for 196x196 icon rendering; `zip` for bundles; `clap` for the CLI; `egui`/`iced`/`tauri` later for a GUI.
- Device (Path B): target `armv7-unknown-linux-musleabihf`; `evdev` for keys; `serialport` or raw termios for ttyS1; `framebuffer` or raw `mmap` on `/dev/fb0`; `tiny-skia` + `fontdue`/`ab_glyph` for rendering; `rusb`-free gadget writes on `/dev/hidg0`.
- Rockchip path only: `rockusb` and `rockfile` crates (Collabora) for maskrom flashing.

## 8. Files produced

- `research/stock-firmware/update_img_header.txt`: decoded ZKSWEV1.0 header of the 5.3.6 app image.
- `research/stock-firmware/res_fs_listing.txt`: file list of the unpacked `res` squashfs.
- `research/stock-firmware/libzkgui_strings.txt`: printable strings from the deck app library (protocol IDs, paths, log messages).
- `research/stock-firmware/binversion`, `md5.txt`: as shipped by Ulanzi Studio.
- Session scratchpad (not persistent) holds the extracted payloads: `C:\Users\aaron\AppData\Local\Temp\claude\c--Users-aaron-Desktop-Ulanzi-D200X\4535dce8-16a4-4ab6-8fcb-8dba701daab3\scratchpad\bin01\UlanziDeckKey` and `...\scratchpad\bin02\res.sqsh`. Copy them into `research/` if you want them kept.

## 9. Progress log

2026-10-29, evening:
- `d200x-tool/` created: Rust CLI with the HID framing, brightness command, and input monitor. Unit tests pass. Live test against the deck succeeded: interface 0 opened, brightness 100 / 40 / 100 accepted. Interface 0 reports usage page 0x000C (Consumer Control), interface 1 is the keyboard. Ulanzi Studio was stopped for the test and relaunched afterwards.
- `adb` installed via winget (Google.PlatformTools, user scope). `adb devices` is empty while the deck runs its app, as expected.
- `research/adb-watch.ps1`: polls for the transient 18d1:d002 interface and runs a read-only dump (`/proc/mtd`, mounts, `/dev`, framebuffer sysfs, `otg_role`, `EasyUI.cfg`, `ps`, `dmesg`) into `research/adb/<timestamp>/`. Start it, then unplug and replug the deck to catch the boot window.
- USBPcap capture needs an elevated shell; this session is not elevated. To capture Ulanzi Studio's handshake: run an admin PowerShell, `USBPcapCMD.exe` (pick the root hub that lists "ulanzi"), output to `research/captures/studio-startup.pcap`, then start Ulanzi Studio.

2026-10-29, 19:56, first ADB dump (`research/adb/20261029_195601/`), triggered by a replug:
- Root `adbd` runs at every boot (`persist.sys.usb.config=adb`, `ro.debuggable=1`) until the deck app switches the gadget to HID, roughly 5 s. Android-style `init` with `stop`/`start` services: `zkdaemon`, `zkdisp` (zkdisplay), `zkswe` (runs `/bin/zkgui`, which loads `/res/lib/libzkgui.so`), `adbd`, `vold`, `logd`. `service.adb.tcp.port=5555` is set but there is no network.
- OS: Linux 4.9.84, "ZKOS / FlyThings V2.1" by zkswe (www.zkswe.com), `ro.firmware=ssd21x_ulanzi`, `ro.product.model=Zkswe_SSD21X_SPINOR`, build date 2025-07-28. FlyThings is a commercial embedded GUI platform with a public IDE/SDK; Ulanzi publishes its TC002 FlyThings app under GPL-3.
- Flash: 32 MiB SPI NOR, `mtdparts=nor0:0x50000(BOOT0),0x1b0000(KERNEL),0x740000(rootfs),0x500000(res),0x90000(config),0x80000(MISC),0x400000(data),0xCB0000(UDISK)`. `rootfs`, `res`, `config` are squashfs; `data` is jffs2 (preferences.json, property/, wallpaper/); `UDISK` is vfat with the factory `update.img` 5.3.6 (identical to the one Ulanzi Studio ships).
- Display: `/dev/fb0` exists (sstarfb), 540x960 portrait, 32 bpp, stride 2160, virtual 540x1920 (double buffered), `rotateScreen: 90` so the UI is 960x540 landscape. The `ui/clock_960_540` directory in the app confirms it. This is not the 1280x720 panel of the RK3308 D200. SigmaStar MI nodes also exist (`mi_disp`, `mi_gfx`, `mi_panel`, `mi_sys`, `mi_ao`), so both fbdev and the MI API are available.
- Input: `/dev/input` is empty. Keys, side buttons and the three knobs all come from the MCU on `/dev/ttyS1` at 115200. The MCU protocol is the main thing left to reverse (logcat on the device, or lift it from `libzkgui.so`/`libeasyui.so`).
- Memory: `LX_MEM=0x3FE0000` (about 64 MiB for Linux), plus an MMA heap of 24 MiB for display/JPEG.
- Backup plan: `research/adb-backup.ps1` waits for the boot window, immediately runs `stop zkdaemon; stop zkswe` so the gadget stays in ADB mode and no recovery reflash can trigger, dumps all eight MTD partitions and the root filesystem, then reboots the deck. Nothing is written to flash.

2026-10-29, 20:05, USB capture of Ulanzi Studio start-up (`research/captures/`, decoded in `research/PROTOCOL.md`):
- New commands confirmed: 0x0003 device-info request with a 0x0303 JSON reply (serial, firmware 5.3.6, hardware SSD210V100, DeviceType "D200"), 0x000d per-key partial update, 0x00e0 key-state feedback after a press, 0x001a one-digit setting after brightness. Manifest keys are `<col>_<row>` on a 5x4 grid, with row 3 holding the knobs and side buttons and `3_2` the wide slot.
- `d200x-tool info` implemented and verified live against the deck.

2026-10-29, 20:16, full root filesystem backup (`research/adb-backup/20261029_201632/fs/`, 471 files):
- Holding ADB open works by killing the deck app by PID from a second, persistent ADB shell session (`research/adb-backup.ps1`). The app is not running yet when ADB first appears; it starts a few seconds later and immediately sets `sys.usb.config=hid`, which init turns into the gadget re-enumeration (see `fs/etc/etc/init.rc`). Neither `stop` (not present in this busybox) nor a premature `setprop ctl.stop` prevents it.
- This adbd predates `exec-out`; partition dumps must be staged on tmpfs and pulled. Script updated; dump pending one more replug.
- Pulled: `/lib` (libeasyui.so, libzkhardware.so, libzknet.so, libzkupgrade.so, SigmaStar MI user libs), `/config/modules/*.ko` (mhal, mi_sys, mi_gfx, mi_disp, mi_panel, mi_ao, fbdev), `/config/board.ini`, `/res` (the app partition), `/system/res/internal`, `/data`.
- The app's MCU link uses the same framework as Ulanzi's open TC002 code (`transfer::SerialTransport`, `KeyProto::MyResponse`, "checksum not match", "mcu ver = %s", `updateMcu`): frames `FF 55 | cmd | len | payload | sum16`. The key/knob command IDs are still unknown; the app logs each received frame, so `research/adb-keylog.ps1` captures logcat while the stock app runs with ADB flipped back on.
- Hardware helper (`libzkhardware.so`) exposes GPIO, I2C, PWM beep, backlight at `/sys/class/backlight/soc:backlight`, thermal zone, and `/dev/watchdog`.

2026-10-29, 20:22, raw flash backup (`research/adb-backup/20261029_202232/`): all eight partitions, verified (res partition equals the Ulanzi Studio image, UDISK holds the factory update.img, BOOT0 holds IPL + U-Boot env with `bootcmd=sf read ... bootm`, `bootdelay=0`, and a "Load IPL_CUST from USB" path).

2026-10-29, 20:26, key log with the stock app running (`research/adb-keylog/20261029_202558/`, decoded in `research/PROTOCOL.md`):
- Keeping ADB alive while the app runs works: a SIGHUP-immune loop on the deck flips `sys.usb.config` back to `adb` after the app sets `hid`. The app keeps running, only its HID channel is gone.
- Full input map recovered (keys 0..13 as a 15-bit matrix mask, side buttons 15/16, knob presses 17..19, knob rotation events).
- **Decisive finding for the approach:** stock standalone mode is a fixed table of about 35 built-in shortcut keywords (`macsave`, `calc`, `volumeSw`, ...) executed over `/dev/hidg0`; there is no generic keystroke action. So "Path A" (Rust configurator driving stock firmware) cannot give real standalone operation. Path B, replacing the app in the `res` partition, is required. Everything needed for it is now in hand: the full flash backup, the vendor flasher's update path (`sys.zkupgrade.flag=255`), the unsigned container format, the input map, fbdev at 960x540, and ADB root at every boot for recovery.

2026-10-29, 20:39 to 20:45, on-device Rust milestone:
- `d200x-device/uartdump`: static armv7 musl Rust binary (built with Rust's bundled lld, no cross gcc) pushed to `/tmp` over ADB and run with the stock app held dead. Captured the MCU protocol on `/dev/ttyS1` at 115200: `FF 55 | cmd u16 LE | len | payload | sum16 BE`; cmd 1 = input state (discrete byte + 15-bit matrix), cmd 2/3/4 = knob 1/2/3 quadrature step. See `research/PROTOCOL.md`.
- `d200x-device/deckproto`: framebuffer test pattern + live MCU decode. Display confirmed working from a plain `/dev/fb0` mmap while the vendor app is dead; rotation determined (3); lit keys matched physical presses. This is the proof that a replacement app can be written in Rust against fbdev + ttyS1 with no vendor libraries, and tested from `/tmp` without flashing.
- Still to do for the on-device side: keyboard reports over `/dev/hidg0` (gadget function `hid` is configured by init; the report descriptor comes from the kernel gadget config), a config channel (either reuse the vendor `hidg1` vendor interface or NCM), rendering (fonts/PNG), persistence in `/data`, and finally packaging as a `ZKSWEV1.0` `res` image with an `EasyUI.cfg` whose `startupLibPath` points at a shim that execs the Rust binary (or replacing `/bin/zkgui`'s role entirely via `/tmp` first).

2026-10-29, 20:53, **standalone proven**: `d200x-device/hidkeys` (static Rust, run from `/tmp`, vendor app dead) read the MCU, drew the grid, switched the gadget to HID itself and typed "abcdefghijklm" plus Enter into Notepad with no host software. Keyboard node is `/dev/hidg1`; `/dev/hidg0` is the 1024-byte vendor channel (host-side descriptors in `research/PROTOCOL.md`). Every building block of a replacement firmware app has now been exercised on the real hardware without touching flash: display, input, USB keyboard output, process launch, persistent storage in `/data`.

What remains is engineering, not discovery:
1. Device app (`deckd`, Rust): config model (pages, keys, knobs, HID actions, macros), icon/text rendering (fbdev, a small raster font + PNG decode), persistence in `/data`, keep-alive-free operation, a host config channel (reuse `/dev/hidg0` with a simple framed protocol; keep the stock-compatible 7c7c framing so `d200x-tool` can talk to both firmwares), sleep/backlight via `/sys/class/backlight/soc:backlight`.
2. Host configurator (`d200x-tool`, Rust): grows from the current CLI; push/pull config over hidg0; later a GUI.
3. Packaging: `EasyUI.cfg` + `startupLibPath` shim or replacing `zkgui`'s role; build a `ZKSWEV1.0` image for mtd3 (`res`, 5 MiB) with the documented header, flashed through the vendor path (`sys.zkupgrade.flag=255`, dir `/tmp`); keep `sys.zkapp.state=running` for `zkdaemon`; factory `update.img` in UDISK stays as the reset-key recovery.
4. Optional later: media keys need a changed gadget descriptor (kernel driver `zkswe_usb`), so volume control may stay host-assisted or use keyboard-only alternatives.

2026-10-29, 21:04, **end-to-end goal reached (unflashed)**: `d200x-device/deckd` 0.1 ran from `/tmp` with the vendor app dead. It rendered a configurable page (labels from a small TTF, colours, highlights), switched the gadget to HID, and answered the Rust host tool on the vendor interface: `d200x-tool info` (deckd identity), `get-config`, `set-config` (ack `{"ok":true}`, read back identical), `brightness`. Keys then typed text, ran a macro, knobs sent arrows, side buttons switched pages, all with no host software. User-confirmed: "works".
- Lesson: Linux sees 33 MB RAM (18 MB free). Parsing the 3 MB CJK font from the rootfs got deckd OOM-killed; a small font in `/data/deckd/font.ttf` is required (the run script pushes `consola.ttf` for testing; ship a free font later).
- Serial: `/sys/class/zkswe_usb/zkswe0/iSerial` holds the ADB placeholder; the stock app gets the real SN elsewhere (to do).
- What is NOT yet done: persistence. deckd lives in `/tmp` and is gone after reboot; the stock app returns. Making it permanent means writing the `res` partition (plan in section 4, Path B): a squashfs with `lib/libzkgui.so` = a shim whose ELF constructor `exec`s `/res/bin/deckd` (so no EasyUI API has to be matched), `etc/EasyUI.cfg`, `bin/deckd`, `font/`, wrapped in a `ZKSWEV1.0` container and applied through the vendor flasher (`sys.zkupgrade.dir`, `sys.zkupgrade.flag=255`). Recovery stays: reset key reflashes the factory image from UDISK; full dumps exist. This is the first step that writes flash and needs an explicit go.

2026-10-29, 21:16, **installed**. `d200x-device/image/mkimage.py` built `out/update.img` (648 KB: squashfs with `lib/libzkgui.so` = zkshim, `bin/deckd`, `etc/EasyUI.cfg`, `font/font.ttf` DejaVu Sans) and `research/adb-install.ps1` applied it through the vendor flasher (`sys.zkupgrade.dir=/tmp`, `sys.zkupgrade.flag=255`). The deck rebooted and came up on deckd from `res`; `d200x-tool info` confirmed. Pre-flash copy of mtd3 in `research/adb-install/20261029_211600/mtd3_before.bin`; factory image still in UDISK for reset-key recovery; ADB root still present at every boot (first ~5 s, before deckd switches the gadget).
- Dry run before flashing (`research/adb-dryrun.ps1`): the vendor loader prefers `/tmp/EasyUI.cfg`, so the complete installed layout was exercised from the RAM disk first.
- zkshim: 2 KB `#![no_std]` cdylib for `armv7-unknown-linux-gnueabihf`, linked with lld, no libc; `.init_array` constructor does a raw `execve` of `/tmp/bin/deckd` then `/res/bin/deckd`.
- Going back to stock: in a boot ADB window push the vendor `update.img` (from Ulanzi Studio's `defProfile/D200X/bin/02/01/bin.zip` or `mtd7_UDISK` extraction) to `/tmp` and set the same two properties, or hold the reset key at power-on.

2026-10-29, 21:22, **restored to stock on request**: `research/adb-install.ps1 -Image research/stock-firmware/update_5.3.6_vendor.img` (the Ulanzi Studio image, byte-identical to the UDISK recovery copy) reflashed `res`; the deck reports firmware 5.3.6 and its real serial again. The round trip stock -> deckd -> stock through the vendor flasher is therefore proven in both directions. The hold loop in the install script now also stops `deckd` so the boot window can be held while deckd is installed.

2026-10-30 (morning): deckd polish without flashing (dry runs from `/tmp`).
- Disassembly tooling (`research/disasm.py`, capstone + pyelftools) recovered the MCU queries (`queryKeyStatus` = `FF 55 01 00 00 01 55`, `queryMcuVersion` = `FF 55 10 00 00 01 64`; this unit's MCU reports `V0.0.05`) and the serial-number path (SPI NOR OTP page 3 via `/dev/oflash`, auth ioctl `0x40046f03` token `0x1d2f2736`, read ioctl `0x80086f04`). The read is still refused by the vendor driver after authentication, so the serial is captured from the stock firmware's USB reply by the install/dry-run scripts and stored in `/data/deckd/serial`; deckd reports it and sets the USB descriptor. Verified: deckd's `info` returns `02D23A045U3672338`.
- deckd: idle dim/off timers, start-up MCU queries, `0x0204` log-tail-over-USB command (`d200x-tool log`) so development no longer needs ADB for diagnostics.
- Installable image regenerated (`d200x-device/image/out/update.img`), not flashed; the deck stays on stock between sessions.

2026-10-30, late morning: tile geometry calibrated on the real deck with a ring test pattern (`d200x-tool calibrate on|off`, concentric 10 px rings per tile). Final values, now the defaults in `deckd/src/config.rs` and part of the config (`layout`): rows y 0/195/390 h 150, columns x 3/199/395/591/787 w 160, wide slot 591,390,356x150, text/icon inset 18 px. Layout changes are live config pushes; no replug needed.

2026-10-30, midday: host-side restructure and GUI.
- `deck-config/`: shared serde model (pages, keys, knobs, actions, layout) used by deckd and the host.
- `d200x-core/`: host library (vendor HID protocol, deckd channel). `d200x-tool/` is now a thin CLI over it.
- `d200x-configurator/`: eframe/egui desktop app. Connect, load/save config from/to the deck, open/save JSON files, page tabs with add/delete, a grid mirroring the deck (click a tile to edit label, colours, icon with upload-and-resize, action: key combination / text / page / next / prev / macro), knob and side-button editors, settings (brightness, dim/off timers, home page, advanced tile layout), deck log window, calibration rings toggle, reboot. Build: `cargo build --release` in `d200x-configurator/`.

2026-10-30, 12:12, **Home Assistant running through the deck** (design in `docs/PLUGINS-AND-HOST-AGENT.md`):
- deckd: `host` actions (plugin/op/args, optional device fallback, red flash on failure or timeout), per-state `looks`, live `value` tiles, agent heartbeat tracking (status line shows "agent" / "no agent"), hello with wanted states and present icons, state map, results. Protocol: deck -> agent 0x0310 action / 0x0311 hello; agent -> deck 0x0210 heartbeat / 0x0211 states / 0x0212 result.
- `d200x-agent`: Windows background process; plugins `system` (open, url, close, media keys via keybd_event) and `homeassistant` (WebSocket auth + `subscribe_events state_changed` + `get_states`, REST service calls; ops toggle/turn_on/turn_off/call_service/scene/script/brightness/volume/media/temperature/number/select_next; state names `ha:<entity>`, `ha:<entity>:text`, `ha:<entity>:attr:<a>`, `ha:<entity>:name`). Config with the token in `%APPDATA%\d200x\agent.json`, user-only ACL; log in `agent.log`.
- `deck-icons`: Material Design Icons (assets/mdi, Apache-2.0) rendered with resvg to tile-size PNGs in any colour; default HA icons per domain/device class.
- Verified live: key toggles `light.buro_aaron_licht` with on/off looks following HA's real state, temperature tile shows `sensor.alpstuga_air_quality_monitor_temperatur`, URL key opens HA, knob 1 steps brightness. Scripts: `research/apply-ha-test.ps1`, config `research/deckd-ha-config.json`.

2026-10-30, afternoon: plugins and configurator redesign.
- Firmware: looks are matched by rules (`deck_config::rule_matches` / `pick_look`): exact value, `*` default, `>x`/`<=x`, `a..b`, `a|b`, `~substring`; exact rules win over comparisons, `*` last.
- Agent plugins: `media` (Windows system media session: play/pause/next/prev/stop, states media:status/title/artist/line/app, album art pushed as `media-art.png`), `obs` (obs-websocket v5 with auth: scenes, record/stream/virtualcam toggles, input mute, source visibility, studio mode; states obs:scene/recording/streaming/virtualcam/mute:<input>), `spotify` (Web API via PKCE with the user's Client ID; play/pause/next/prev/shuffle/repeat/like/volume/play uri; states spotify:playing/title/artist/album/line/shuffle/repeat/liked/volume/device), `discord` (RPC over the IPC pipe with the user's app id/secret, authorize once; mute/deafen/ptt toggles; states discord:mute/deaf/ptt/connected). Agent config gained `obs`, `spotify`, `discord` sections.
- Configurator rewritten (`app.rs`, `ui_canvas.rs`, `ui_inspector.rs`, `ui_library.rs`, `ui_windows.rs`, `integrations.rs`): top bar, pages sidebar, deck canvas with real icons and knobs/side buttons, inspector with Appearance / Action (hotkey recorder, macro text, agent action editor) / States & looks (rule editor with per-rule icon, colour, label and Material icon picker), bottom library with per-plugin panels (Keyboard presets, Pages, System with file picker, Media, Home Assistant entity picker, OBS setup + scene/input lists, Spotify setup + PKCE login, Discord setup). Icons are rendered locally into `%APPDATA%\d200x\icons` and uploaded on "Save to deck". Firmware window installs deckd or restores stock through the install script.

2026-10-30, later: visual features.
- Firmware: per-key `progress` (state, min/max, style `border` ring clockwise from top/right/bottom/left or `fill` from a side, colour, width); press animation (tile insets 6 px while pressed and springs back over 220 ms, 60 fps redraw while animating); page `background` image (960x540 from the icon store) with tiles whose colour is `none` drawn transparent; `Action::Screen { image, seconds }` shows a full-screen image until any key press or timeout.
- Configurator: Appearance tab gained transparent background and the progress editor with presets (brightness ring for lights, volume ring, temperature fill, generic ring/fill); page background picker in Settings; "show full-screen image" action kind and a Pages-library button that picks the image; "Subpage + back" preset; Home Assistant rows gained "toggle + ring"; the Material icon picker shows rendered thumbnails in the chosen colour; the canvas draws the physical control row (two buttons then three knobs, evenly spaced below the screen) and hints progress styles.

2026-10-30, 14:06: application pages and focus-based page switching.
- Model: `Page.app` = foreground process pattern (`photoshop.exe|lightroom.exe`, case-insensitive substring); `deck_config::app_matches`; `wanted_states()` adds `system:app` whenever any page has a pattern.
- Agent: `system::Foreground` thread polls `GetForegroundWindow` every 400 ms (process image name via `QueryFullProcessImageNameW`, window title) and publishes `system:app` / `system:title`; the session loop pushes them like any other state.
- Firmware: on a `system:app` change, `auto_switch` shows the first page in page order whose pattern matches and remembers the page it came from; when no page matches it returns there. A manual page action (page / next / prev) cancels the auto return. Numpad key names (`numpad0..9`, `numpad+` ...) added to the HID key table.
- Configurator: new library tab "Apps" with 15 presets (VS Code, Photoshop, Lightroom, Premiere, DaVinci Resolve, Blender, Browser, Teams, Zoom, OBS, Discord, Spotify, Excel, Explorer): left click creates/replaces the page with 13 hotkeys and the app pattern, right click also puts a "Go to" key on the selected slot. Settings shows "Page ... auto-switch" with the pattern field, a running-application picker (`tasklist /V`, refreshed in a background thread) and a red note when the agent is not running.
- Image `out/update.img` rebuilt (700988 bytes); installer armed for the next replug.

2026-10-30, 14:55: AgentDeck support (github.com/puritysb/AgentDeck, the coding-agent session monitor; its own Ulanzi plugin needs Ulanzi Studio, so deckd gets a native one).
- Agent plugin `agentdeck` (`d200x-agent/src/agentdeck.rs`): WebSocket client to the local daemon (port from `~/.agentdeck/daemon.json`, default 9120, loopback needs no token), registers as `ulanzi-plugin`, consumes `sessions_list` / `state_update` / `prompt_options` / `usage_update`. States: `agentdeck:state|count|waiting|connected`, per roster slot 0..7 `agentdeck:slot:N:state|name|line|tool|question`, selected session `agentdeck:sel:*` incl. `sel:opt:I` option labels, `agentdeck:usage:5h|7d|tokens`. Ops: select {slot}, select_next/prev, approve, deny (permission gate -> `permission_decision` with the row's requestId; prompt -> first/last option via `select_option` with question echo, or `respond` y/n on managed PTY sessions), option {index}, stop (interrupt; observed sessions wrapped in `session_command`), escape, prompt {text}, mode, review, voice, refresh. Managed sessions get `focus_session` first.
- Firmware: `draw_label` splits on an explicit newline (two fitted lines) so "project / activity" tiles render; image rebuilt (installer still armed for the next replug).
- Configurator: library tab "AgentDeck" with a whole-page preset (8 session tiles with idle/busy/waiting/off looks, Yes/No, option 2, Stop, selected-session tile with 5-hour usage ring, session knob), single tiles, option tiles, prompt sender, daemon port setting. Config mirror gained `agentdeck.port`; default plugin list includes `agentdeck`.
- Verified live: agent log "agentdeck connected on 9120"; after pushing a page built the same way as the preset (`agentdeck` page appended to the deck config, icons uploaded) deckd's hello asked for 21 states and the agent pushed them (0x0211). This Claude Code session itself appears as an observed session on the deck.

2026-10-30, 15:30: AgentDeck tiles now come from AgentDeck's own renderer (the user wanted the real icons and animations, not approximations).
- `d200x-agent/bridge/bridge.mjs` (Node, MIT port of plugin-ulanzi/src/app.ts + state-store/session-order/deck-signature) connects to the daemon, runs `buildSessionDeck` from the published `@agentdeck/shared` 1.9.6 package (list view, detail view with options/permission/quick actions, usage gauges) and rasterises the SVG cells with `@resvg/resvg-wasm` and the plugin's fonts (IBM Plex Sans, JetBrains Mono in `bridge/resources/fonts`). Animated cells follow the plugin's schedule: 24 frames per `SESSION_SLOT_ANIM_CYCLE`, 145 ms per frame, phase-shifted per key (stride 7). Protocol to the agent: JSON lines (`positions`/`press`/`release` in, `tile`/`anim`/`log` out).
- Agent: the `agentdeck` plugin spawns the bridge (needs `node` on PATH), forwards the deck's wanted `agentdeck:k<i>` states as tile positions, uploads rendered tiles to the deck as `ram-ad<i>-<sig>.png` (+ `-fNN.png` frames) and points the key's tile state at them (`name` or `name|frames|delay`). Op `key {key}` relays presses (and releases, for the voice hold) to the bridge.
- Firmware: `KeyDef.tile` = state whose value names a full-tile image (static or animation); images whose name starts with `ram-` live in `/tmp/deckd-icons` (tmpfs, pruned at 8 MB, never flash); the icon cache is capped at 48 decoded images; animated tiles tick the redraw at the frame delay; label text honours an explicit newline. Image rebuilt.
- Configurator: AgentDeck tab preset "AgentDeck (official)" = page of 14 tile keys; preview shows "AgentDeck" on such keys. The deckd-native tiles remain as "Simple tiles page".
- The deck config now carries the official page; it renders after the next replug (installer armed with the new image).

2026-10-30, 20:15: Minecraft icons, full-screen effects, Home Assistant automations.
- `deck-config`: `KeyDef.effect` (played on every press, next to the key's action) and `Action::Effect`; `Effect::LevelUp {ms}` (blocky XP-green wave left to right with a short flash), `Effect::Wave {color, ms, from, blocky}`, `Effect::Frames {name, frames, delay_ms}` (PNG frames blended over the page). `Action::Screen` gained `frames` + `delay_ms`: an animation fills the deck, plays once when `seconds` is 0, loops otherwise. Old configs load unchanged (serde defaults, tests).
- Firmware: effects are drawn after the page at 16 ms ticks (procedural wave: <0.5 ms per frame on the PC, verified with a host-side render of `render.rs`); frame animations decode each 480x270 PNG on the fly (never cached; a 960x540 frame would be 2 MB of RAM). Image rebuilt: `d200x-device/image/out/update.img`, not yet flashed.
- `deck-icons`: `import_minecraft(jar, out)` extracts `assets/minecraft/textures/item/*.png` from the player's own client jar (`find_minecraft_jar` picks the newest release under `%APPDATA%\.minecraft\versions`), numbered animation strips collapse to frame 00; `render_minecraft` scales 16 px art by an integer factor with nearest-neighbour sampling (icon files `mc-<item>.png`). `render_animation` turns a GIF into at most 40 PNG frames at 480x270 (`<base>-fNN.png`). Binary `import-minecraft` for the command line. 689 icons imported from 26.3 (Mojang assets, local use only, see `assets/minecraft/README.md`).
- Configurator: "Minecraft item..." picker (Appearance and per-look), library tab "Minecraft" (import, level-up on the selected key / the whole page, starter keys), Pages tab "Play animation..." (GIF) and "Level-up effect", Appearance "Press effect" editor (level up / colour wave / GIF frames), action kinds "show full-screen image / animation" with frames and "full-screen effect"; canvas marks keys with an effect with a green "fx".
- Level-up refined (20:40): rings roll outwards from the pressed key's centre (blocky 20 px cells, white front, glow at the start); each ring takes 700 ms and a new one spawns every 250 ms while the duration allows (`ring_count`); the key that fired the effect is redrawn as a blank slab in front of the rings (`draw_blank_tile`). `Action::Effect` keys start at their own key. Configurator persists the working config at `%APPDATA%\d200x\config.json` (autosave 3 s after edits, on close, after load/save to deck) and restores it at start; with no local copy it loads the deck's config on connect.
- Key clipboard (22:10): copy / cut / paste of whole keys in the configurator (`App::clipboard`): Ctrl+C / Ctrl+X / Ctrl+V on the deck preview while a key is selected and no text field has focus, right-click menu on a tile (Copy, Cut, Paste, Clear), copy / paste buttons in the key inspector header. Works across pages; icons are already in the store.
- Wide key (21:55): the wide slot's label now honours size, align and valign in the area above the knob strip (default: top), on the deck and in the preview; before, slot 13 ignored all text styling.
- Vertical alignment (21:45): `KeyDef.valign` top/middle/bottom (None = middle without icon, under the icon with one; with an icon "top" puts the text strip above it, "middle" overlays it). Configurator combo + preview.
- Label templates and text style (21:30): a label may contain `{}` (the key's `value` state) or `{state|fmt}` placeholders (`deck_config::render_label`; fmt `0`/`1`/`2` = decimals of the first number, `int`, `upper`, `lower`; missing state = "-"); a plain label is still replaced by `value`. `wanted_states` collects placeholder names from key and look labels. `KeyDef.font_size` (fixed px, None = auto-fit), `align` (left/center/right), `font` (a `font-xxx.ttf` uploaded through the icon channel, cached 4 at a time in the renderer; the icon upload handler drops the font cache on `.ttf`). Configurator: Appearance shows text size, alignment, font combo + "TTF file..." (max 1.5 MB), template hint; preview honours size and alignment.
- Firmware window fix (21:15): the window never showed "[finished]" because the adb server daemon started by `adb-install.ps1` inherited the script's stdout pipe, so the configurator's EOF-based reader never ended. Now the window follows the script's process exit (readers run on their own threads) and the script starts the adb server detached (`Start-Process ... -WindowStyle Hidden -Wait`).
- Level-up speed (21:05): `Effect::LevelUp { ms, wave_ms, every_ms }`: `wave_ms` = time one ring needs to cross the screen (speed), `every_ms` = interval between rings, `Effect::ring_count`. The key that fired the effect is now redrawn in full (icon, label, ring) in front of the waves (`draw_tile` split out of `draw_page`, `draw_tile_over`). Configurator: wave speed and interval sliders with the resulting wave count.
- State-driven background (20:55): `KeyDef.color_state` names a state whose value, when it parses as a colour (`#rrggbb`, `r,g,b`, `[r, g, b]`; `deck_config::parse_color_value`), replaces the tile's background (after looks). HA lights: `ha:light.x:attr:rgb_color` (serialised as `[r, g, b]` by the agent). Configurator: Appearance "Background from state" with a "light colour" shortcut; HA tab "toggle + colour" for lights (colour + thin white brightness ring). Firmware rebuilt.
- Agent HA ops: `toggle`/`turn_on`/`turn_off` call `input_boolean.*` and `automation.*` for those domains (an automation toggle enables/disables it), new `trigger` = `automation.trigger` (skip_condition true unless given). Configurator HA tab: quick filters (automations, booleans, scripts, lights, switches), "run" key for automations and scripts, "on/off key" for automations, "toggle key" for input_boolean with live looks.

2026-11-01, 16:30: Spotify / Discord connection fixes, on/off progress.
- Root cause of both "plugin not configured" reports: the agent reads `agent.json` only at start, and it had been running since before the settings were saved (agent start 14:12Z, config written 15:11Z), so the Discord consent prompt could never appear. Every settings save in the configurator now restarts a running agent (`restart_agent`). After the restart Discord authorized at once and Spotify polled cleanly.
- Spotify login: the callback listener served exactly one connection and had no timeout despite its comment, so a browser preconnect or a stray request ended or hung the login, and the next click failed on the busy port. It now serves the port for up to 3 minutes, ignores everything but `/callback` with the matching `state`, reports `error=`, and a new click replaces a waiting login. The token is written to `agent.json` from the login thread and the result is polled every frame, not only while the Spotify tab is open.
- Stale tokens: the configurator held its own copy of `agent.json` and wrote it back over tokens the agent had stored (Discord access token, rotated Spotify refresh token). `save_agent_config` now keeps the tokens on disk unless the client id changed.
- Discord: commands sat behind a blocking pipe read and only went out when the next message arrived. The loop now polls with `PeekNamedPipe`; the unused second pipe connection is gone.
- `Progress::fraction` (deck-config): on/off words (on/off, true/false, yes/no, open/closed, playing/paused, ...) show the indicator full or empty whatever min and max are; numbers work as before. Used by deckd, so it needs a reflash. Flashed 16:39.
- Progress ring redrawn (16:50, `deckd/src/ring.rs`): a band between two rounded rectangles instead of a square walk along the edge, so inner and outer corners are round and antialiased; the sweep follows the band's own length (also on the wide slot). At 0 nothing is drawn, not even the dim track. Checked on the PC (`rustc --test src/ring.rs`, rendered sample tiles). Image rebuilt, not yet flashed.
- Colour field (`d200x-configurator/src/ui_color.rs`): every colour in the configurator now uses `color_field`, egui's picker plus a paste button and Ctrl+V (accepts `#rrggbb`, `rrggbb` and the `r, g, b` text of the picker's copy button) and up to 16 recent colours, kept in `%APPDATA%\d200x\colors.json`. A colour counts as picked when its popup closes. Ctrl+V on the deck preview does not paste a key while a colour popup is open.
- `Progress.color_state`: the ring or fill takes its colour from a state (same formats as `KeyDef.color_state`, e.g. `ha:light.x:attr:rgb_color`) and falls back to `color` while the state has no colour. Configurator: "colour from state" with a "light colour" shortcut in the progress editor. Firmware change, image rebuilt, not yet flashed.
- State explorer (17:15): the agent answers every connection on `127.0.0.1:8898` with a JSON snapshot of all states (`{plugins, states}`, about 5000 states / 350 KB with 689 HA entities, no tokens). Configurator window "States" (top bar, or "explore..." next to a key's state): `states.rs` merges a catalogue of what each plugin offers (meaning, possible values) with the snapshot, HA by entity with possible values from the domain or the entity's own lists (`options`, `hvac_modes`, `effect_list`, ...); `ui_states.rs` applies a state to the selected key as state, label, background, ring or ring colour and adds looks from the possible values. `discord:channel` was documented but never set: removed from the docs.
- State picker: every state field of a key (state, label, background, ring, ring colour) has a "pick" button that opens the explorer for that field (`StateField`, `pick_for`); "Use for ..." or a double-click fills the field and closes the window. An empty field starts at the key's own state.

2026-11-01, 17:30: firmware speed. Numbers measured on the deck from `/tmp` in the boot ADB window, with the hold loop keeping one core busy, so a running deck does better.
- `/proc/cpuinfo`: two Cortex-A7 (part 0xc07), `neon vfpv4 idiva vfpd32`. deckd used one core and had no NEON. Parsed DejaVu Sans takes 7 MB of RAM.
- What it cost before: `draw_label_styled` cloned the whole parsed font per label (130 ms each, and the second copy can get deckd OOM-killed: a benchmark with 12 MB free died on its first page); rotating the canvas into the framebuffer 52 ms; FBIOPAN_DISPLAY 16.5 ms, it blocks until vsync, on the input thread; the AgentDeck page 113 ms per redraw, 14 PNGs decoded again even for a press animation; every heartbeat, state push and icon upload written to JFFS2 (up to 8 ms each, flash wear, unbounded file on the config partition); `setprop` spawn 4.7 ms.
- Now: input stays on the main thread; `display.rs` renders on the second core and takes the newest frame from a one-slot mailbox, so a slow frame is skipped, not queued. `build_page` turns config and states into plain `TileLook`s (0.13 ms); `compose.rs` keeps the page composed and repaints only tiles whose look changed, effects go over a copy of it. A tile is clipped to its slot, a label too large for it no longer spills onto the neighbours. Dirty rects reach both framebuffer pages (`Fb::blit`: this frame's plus the last frame's). The rotate works in 32x32 squares through a small buffer. Glyphs, the current picture of each tile and the scaled background are cached. Animated tiles step one after the other (`now + slot * delay / 14`), so 14 decodes no longer land on one frame. Keyboard output and host messages have their own threads (`KeyQueue`, `Vendor::send` drops while the host is not reading), macros run from the main loop and no longer freeze the deck during their delays. Build: opt-level 3, `target-cpu=cortex-a7`, `+neon` (image 1.16 MB binary, was 0.8).
- Log: `trace` goes to a 60 KB memory tail only, which is what `d200x-tool log` shows; `line` also writes `deckd.log`, for start-up and rare events. The render thread adds a `perf:` line every 10 s while frames are shown.
- Results on the deck: press animation 88 ms + 130 ms per label -> 1.7 ms draw; one tile blit well under a frame; AgentDeck step 113 ms -> 5.8 ms per tile; unchanged frame 0.01 ms and no flip; full page repaint 42 ms; full-screen effect about 28 ms per frame (compose 14, blit 14), so 30 fps; peak RSS 13.8 MB, was 18.8 MB and climbing when the OOM killer stopped it. `update.img` 824 KB, was 746 KB.
- Not yet tried with fingers: key presses, knobs, keyboard output and macros on the new threads. The test deckd ran from `/tmp`, nothing was flashed.
- Checked: 60 scenes pixel-identical between the old and new renderer (all pages of the live config plus a test page with background, text styles, fonts, knobs, effects); 1500 random steps through the dirty rects and both pages equal a fresh full paint, and two deliberately broken variants were caught; framebuffer read back on the deck: rotation and page flips correct.

## 10. Sources

- https://github.com/riptidewave93/d200-buildroot (docs: hardware, install, recovery, linux, u-boot)
- https://github.com/riptidewave93/d200-deck (docs: api.md, config.md)
- https://github.com/atomicstack/tc002-customisation (FIRMWARE.md, TEARDOWN.md, issue 3)
- https://github.com/kastro5/opendeck-ulanzi-d200
- https://jcalado.com/posts/ulanzi-d200-companion/
- https://hackaday.com/2025/10/21/budget-stream-deck-clone-is-ripe-for-hacking/
- https://github.com/puritysb/AgentDeck/issues/174 (D200X fw 5.3.6, hardware ID SSD210V100)
- https://github.com/Misyuk-T/ulanzi-d200-recovery and https://github.com/CHADREX/ulanzi-d200-recovery
- http://linux-chenxing.org/pioneer3/
- https://www.cnx-software.com/2021/03/19/sigmastar-ssd210-tiny-dual-core-arm-cortex-a7-soc-64mb-ram-rgb-display/
- https://wx.comake.online/doc/ds82ff82j7jsd9-SSD220/customer/development/software/Px/en/sys/P3/usb%20update.html
- https://github.com/collabora/rockchiprs
