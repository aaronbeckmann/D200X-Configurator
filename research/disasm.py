"""Disassemble selected exported functions of the vendor deck app (libzkgui.so, ARM Thumb-2)
to recover MCU command IDs. Usage: python -I disasm.py [symbol-substring ...]
"""
import sys
import os
from elftools.elf.elffile import ELFFile
from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB, CS_MODE_ARM

HERE = os.path.dirname(os.path.abspath(__file__))
SO = os.path.join(HERE, "adb-backup", "20261029_201632", "fs", "res", "res", "lib", "libzkgui.so")
wanted = sys.argv[1:] or ["queryMcuVersion", "queryKeyStatus", "firmwareSN", "readyUpdateMcu", "sendFileSize"]

f = open(SO, "rb")
elf = ELFFile(f)
dynsym = elf.get_section_by_name(".dynsym")
syms = {}
for s in dynsym.iter_symbols():
    if s["st_value"] and s["st_size"]:
        syms[s.name] = (s["st_value"], s["st_size"], s["st_info"]["type"])
by_addr = sorted(((v[0] & ~1, k) for k, v in syms.items()))

def sym_at(addr):
    # nearest symbol at or below addr
    lo, hi = 0, len(by_addr)
    best = None
    for a, n in by_addr:
        if a <= addr:
            best = (a, n)
        else:
            break
    if best and addr - best[0] < 0x2000:
        return f"{best[1]}+0x{addr-best[0]:x}" if addr != best[0] else best[1]
    return None

text = elf.get_section_by_name(".text")
rodata = elf.get_section_by_name(".rodata")
def read(addr, n):
    for sec in (text, rodata):
        start = sec["sh_addr"]
        if start <= addr < start + sec["sh_size"]:
            off = addr - start
            return sec.data()[off:off + n]
    return b""

md_t = Cs(CS_ARCH_ARM, CS_MODE_THUMB)
md_a = Cs(CS_ARCH_ARM, CS_MODE_ARM)
md_t.detail = False

for name, (addr, size, typ) in sorted(syms.items(), key=lambda kv: kv[1][0]):
    if not any(w in name for w in wanted):
        continue
    thumb = addr & 1
    base = addr & ~1
    code = read(base, size)
    print(f"\n==== {name} @ 0x{base:x} size {size} ({'thumb' if thumb else 'arm'})")
    md = md_t if thumb else md_a
    for ins in md.disasm(code, base):
        note = ""
        if ins.mnemonic in ("bl", "blx", "b", "b.w", "bl.w") and ins.op_str.startswith("#0x"):
            tgt = int(ins.op_str[1:], 16)
            s = sym_at(tgt)
            if s:
                note = f"   ; -> {s}"
        if ins.mnemonic.startswith("mov") and "#" in ins.op_str:
            note += "   ; imm"
        print(f"  {ins.address:08x}  {ins.mnemonic:8} {ins.op_str}{note}")
