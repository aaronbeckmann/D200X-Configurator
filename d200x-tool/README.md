# d200x-tool

Rust host tool for the Ulanzi D200 / D200H / D200X vendor HID protocol. Verified on 2026-10-29 against a D200X, hardware revision 02 (SigmaStar SSD210), firmware 5.3.6.

```
cargo build
target\debug\d200x-tool.exe list              # enumerate the deck's HID interfaces
target\debug\d200x-tool.exe info              # serial, firmware, hardware version (cmd 0x0003 -> 0x0303)
target\debug\d200x-tool.exe brightness 60     # 0-100
target\debug\d200x-tool.exe monitor           # print key / side button / dial events
```

Ulanzi Studio holds interface 0 exclusively. Quit it (tray icon) before `brightness` or `monitor`.

## Layout

- `src/protocol.rs`: framing (`7c 7c | cmd u16 BE | len u32 LE | payload`, 1024-byte reports), command IDs, input indices, the ZIP boundary-byte bug check. Unit tested.
- `src/device.rs`: hidapi transport. Opens interface 0 (usage page 0x000C, usage 0x0001). Prepends the 0x00 report-ID byte that hidapi requires on Windows.
- `src/main.rs`: CLI.

## Next

- `set-buttons`: build the icon ZIP (`manifest.json` + 196x196 PNG/JPEG per key, `3_2` wide slot with `SmallViewMode`) and pad it so no 0x00/0x7c byte lands on a report boundary.
- Keep-alive: resend brightness every 5 s or the deck drops to its boot logo after roughly a minute.
- Decode the `0x0101/0x0102` input payloads once real events are captured with `monitor`.
