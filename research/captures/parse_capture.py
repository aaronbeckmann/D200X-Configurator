"""Decode the deck's vendor HID frames from the tshark TSV dump.

Reads studio-startup-interrupt.tsv (frame, time, src, dst, ep, len, capdata, usbhid.data, data)
and writes decoded payloads into payloads/.
"""
import os
import sys

here = os.path.dirname(os.path.abspath(__file__))
tsv = os.path.join(here, "studio-startup-interrupt.tsv")
outdir = os.path.join(here, "payloads")
os.makedirs(outdir, exist_ok=True)

rows = []
with open(tsv, encoding="utf-8-sig") as f:
    for line in f:
        p = line.rstrip("\n").split("\t")
        if len(p) < 8:
            continue
        data = bytes.fromhex(p[7]) if p[7] else b""
        rows.append((int(p[0]), float(p[1]), p[2], p[3], p[4], data))

def ascii(b, n=80):
    return "".join(chr(c) if 32 <= c < 127 else "." for c in b[:n])

print(f"{len(rows)} interrupt packets with payload")

# Host -> deck: reassemble by declared length.
pending = None  # (cmd, need, buf, start_frame, start_time)
messages = []
for frame, t, src, dst, ep, data in rows:
    if src != "host":
        continue
    # USBPcap on Windows prepends the report ID byte 0x00 to the 1024-byte report
    if len(data) == 1025 and data[0] == 0:
        data = data[1:]
    if pending is None:
        if data[:2] != b"\x7c\x7c":
            print(f"frame {frame}: unexpected start {data[:8].hex()}")
            continue
        cmd = int.from_bytes(data[2:4], "big")
        need = int.from_bytes(data[4:8], "little")
        buf = bytearray(data[8:8 + need])
        if len(buf) >= need:
            messages.append((frame, t, ep, cmd, bytes(buf)))
        else:
            pending = (cmd, need, buf, frame, t, ep)
    else:
        cmd, need, buf, f0, t0, ep0 = pending
        take = need - len(buf)
        buf.extend(data[:take])
        if len(buf) >= need:
            messages.append((f0, t0, ep0, cmd, bytes(buf)))
            pending = None
        else:
            pending = (cmd, need, buf, f0, t0, ep0)

names = {1: "SET_BUTTONS", 3: "CMD_0003", 6: "SET_SMALL_WINDOW", 0xa: "SET_BRIGHTNESS", 0xb: "SET_LABEL_STYLE"}
print("\n=== host -> deck messages")
for i, (frame, t, ep, cmd, payload) in enumerate(messages):
    name = names.get(cmd, f"CMD_{cmd:04x}")
    print(f"#{i:3} frame {frame:4} t={t:8.3f} ep{ep} {name:17} len={len(payload):7}  {ascii(payload)}")
    if len(payload) > 0:
        ext = "zip" if payload[:2] == b"PK" else ("json" if payload[:1] in b"{[" else "bin")
        with open(os.path.join(outdir, f"{i:03}_{name}_{frame}.{ext}"), "wb") as f:
            f.write(payload)

print("\n=== deck -> host packets with payload")
for frame, t, src, dst, ep, data in rows:
    if src == "host" or not data:
        continue
    print(f"frame {frame:4} t={t:8.3f} {src} ep{ep} len={len(data)} {data[:24].hex()}  {ascii(data, 24)}")

# Unique command summary
from collections import Counter
c = Counter(names.get(cmd, f"CMD_{cmd:04x}") for _, _, _, cmd, _ in messages)
print("\n=== counts:", dict(c))
