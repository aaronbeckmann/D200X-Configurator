"""Build the deckd `res` partition image for the Ulanzi D200X rev 02.

Steps:
  1. stage/   : etc/EasyUI.cfg, lib/libzkgui.so (zkshim), bin/deckd, font/font.ttf, README
  2. res.sqsh : squashfs 4.0, xz, 128 KiB blocks, all-root (mksquashfs in WSL)
  3. update.img: ZKSWEV1.0 container for mtd index 3 (`res`), device code 0xaa550606,
                 device flag 0 (SPI NOR), header CRC-32, payload MD5, stashed first 16 bytes.
                 Layout per atomicstack/tc002-customisation FIRMWARE.md and verified against
                 Ulanzi's own 5.3.6 update.img (research/stock-firmware/update_img_header.txt).

Run from Windows:  python -I mkimage.py
"""
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
STAGE = os.path.join(HERE, "stage")
OUT = os.path.join(HERE, "out")
VENDOR_IMG = os.path.join(ROOT, "research", "adb-backup", "20261029_202232", "mtd7_UDISK.bin")  # holds the factory update.img

deckd = os.path.join(ROOT, "d200x-device", "deckd", "target", "armv7-unknown-linux-musleabihf", "release", "deckd")
shim = os.path.join(ROOT, "d200x-device", "zkshim", "target", "armv7-unknown-linux-gnueabihf", "release", "libzkshim.so")
font = os.path.join(HERE, "font", "font.ttf")
font_lic = os.path.join(HERE, "font", "LICENSE-DejaVu.txt")
cfg = os.path.join(HERE, "EasyUI.cfg")

for p in (deckd, shim, font, cfg):
    if not os.path.exists(p):
        sys.exit(f"missing {p}")

# 1. stage
shutil.rmtree(STAGE, ignore_errors=True)
for d in ("etc", "lib", "bin", "font"):
    os.makedirs(os.path.join(STAGE, d))
shutil.copy(cfg, os.path.join(STAGE, "etc", "EasyUI.cfg"))
shutil.copy(shim, os.path.join(STAGE, "lib", "libzkgui.so"))
shutil.copy(deckd, os.path.join(STAGE, "bin", "deckd"))
shutil.copy(font, os.path.join(STAGE, "font", "font.ttf"))
if os.path.exists(font_lic):
    shutil.copy(font_lic, os.path.join(STAGE, "font", "LICENSE-DejaVu.txt"))
with open(os.path.join(STAGE, "README"), "w") as f:
    f.write("deckd open firmware for Ulanzi D200X rev 02 (SSD210). lib/libzkgui.so is a shim that execs bin/deckd.\n"
            "Recovery: hold the reset key at power-on to reflash the factory update.img from UDISK.\n")
os.makedirs(OUT, exist_ok=True)

# 2. squashfs via WSL
def wsl(p):
    p = os.path.abspath(p).replace("\\", "/")
    return "/mnt/" + p[0].lower() + p[2:]

sqsh = os.path.join(OUT, "res.sqsh")
if os.path.exists(sqsh):
    os.remove(sqsh)
cmd = ["wsl", "-d", "Ubuntu", "--", "bash", "-c",
       f"chmod 755 '{wsl(STAGE)}/bin/deckd' '{wsl(STAGE)}/lib/libzkgui.so'; "
       f"mksquashfs '{wsl(STAGE)}' '{wsl(sqsh)}' -comp xz -b 131072 -no-xattrs -all-root -noappend -quiet"]
r = subprocess.run(cmd, capture_output=True, text=True)
if r.returncode != 0 or not os.path.exists(sqsh):
    sys.exit("mksquashfs failed:\n" + r.stdout + r.stderr)
img = open(sqsh, "rb").read()
assert img[:4] == b"hsqs", "not a squashfs"
# pad to 4 KiB
pad = (-len(img)) % 4096
img += b"\xff" * pad
print(f"squashfs {len(img)} bytes (padded)")
PART_SIZE = 0x500000
assert len(img) <= PART_SIZE, "image larger than the 5 MiB res partition"

# 3. container
vendor = open(VENDOR_IMG, "rb").read()
vpos = vendor.find(b"ZKSWEV1.0-180127")
assert vpos >= 0, "vendor header not found in UDISK dump"
vhdr = vendor[vpos:vpos + 0x24c]

hdr = bytearray(0x23c)
hdr[0x000:0x010] = b"ZKSWEV1.0-180127"
hdr[0x010] = 0x30            # prefix length
hdr[0x011] = 1               # entry count
hdr[0x012] = 0x30            # info block offset
hdr[0x013] = vhdr[0x013]     # reserved (0x23 in vendor image)
# partition entry at 0x14
e = 0x14
hdr[e] = 3                   # mtd index: res
hdr[e + 1:e + 4] = vhdr[e + 1:e + 4]  # reserved bytes as in vendor image
struct.pack_into("<I", hdr, e + 4, 0x23c)       # payload offset
struct.pack_into("<I", hdr, e + 8, len(img))    # image size
hdr[e + 12:e + 28] = img[:16]                   # stashed first 16 bytes
# info block
struct.pack_into("<I", hdr, 0x30, 0x20c)
hdr[0x34] = vhdr[0x34]       # 0x02
struct.pack_into("<I", hdr, 0x35, 0xaa550606)   # device code
hdr[0x39] = 0                # device flag: SPI NOR
hdr[0x3a] = vhdr[0x3a]
hdr[0x3b:0x238] = vhdr[0x3b:0x238]              # opaque block, copied from vendor image
crc = zlib.crc32(bytes(hdr[:0x238])) & 0xffffffff
struct.pack_into("<I", hdr, 0x238, crc)
md5 = hashlib.md5(img).digest()
payload = md5 + img[16:]
update = bytes(hdr) + payload
out_path = os.path.join(OUT, "update.img")
open(out_path, "wb").write(update)
open(os.path.join(OUT, "md5.txt"), "w").write(hashlib.md5(update).hexdigest())
open(os.path.join(OUT, "binversion"), "w").write("0.1.0")
print(f"wrote {out_path} ({len(update)} bytes), header crc 0x{crc:08x}, payload md5 {md5.hex()}")

# self-check: re-parse like the flasher
d = update
assert d[:9] == b"ZKSWEV1.0"
assert struct.unpack_from("<I", d, 0x35)[0] == 0xaa550606
assert zlib.crc32(d[:0x238]) & 0xffffffff == struct.unpack_from("<I", d, 0x238)[0]
off, size = struct.unpack_from("<II", d, 0x14 + 4)
first16 = d[0x14 + 12:0x14 + 28]
rebuilt = first16 + d[off + 16:off + size]
assert hashlib.md5(rebuilt).digest() == d[0x23c:0x24c], "md5 mismatch"
assert rebuilt == img
# the same check against the vendor image proves the md5 formula
voff, vsize = struct.unpack_from("<II", vendor, vpos + 0x14 + 4)
vfirst = vendor[vpos + 0x14 + 12:vpos + 0x14 + 28]
vimg = vfirst + vendor[vpos + voff + 16:vpos + voff + vsize]
assert hashlib.md5(vimg).digest() == vendor[vpos + 0x23c:vpos + 0x24c], "vendor md5 formula mismatch"
assert zlib.crc32(vendor[vpos:vpos + 0x238]) & 0xffffffff == struct.unpack_from("<I", vendor, vpos + 0x238)[0], "vendor crc formula mismatch"
print("self-check OK: header CRC, payload MD5 and stash layout match the vendor image's own values")
