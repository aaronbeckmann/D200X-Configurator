//! deckproto: proves the two on-device building blocks for a replacement app.
//!
//! 1. Draws a test pattern on /dev/fb0 (540x960 portrait panel used as 960x540
//!    landscape): a 5x3 key grid plus a wide slot, with corner markers so the
//!    rotation can be judged from a photo (red = logical top-left, green =
//!    top-right, blue = bottom-left).
//! 2. Reads the MCU on /dev/ttyS1 (115200) and highlights pressed keys, side
//!    buttons and knob presses; knob rotation moves a bar in the wide slot.
//!
//! Usage: deckproto <seconds> [rotation 0..3]
//! Logs every decoded event to stdout.

use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;
const FBIOPAN_DISPLAY: libc::c_ulong = 0x4606;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FbBitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FbVarScreeninfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitfield,
    green: FbBitfield,
    blue: FbBitfield,
    transp: FbBitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FbFixScreeninfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    type_: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: libc::c_ulong,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

struct Fb {
    file: std::fs::File,
    mem: *mut u8,
    len: usize,
    var: FbVarScreeninfo,
    stride: usize,
}

impl Fb {
    fn open() -> Fb {
        let file = std::fs::OpenOptions::new().read(true).write(true).open("/dev/fb0").expect("open /dev/fb0");
        let fd = file.as_raw_fd();
        let mut var = FbVarScreeninfo::default();
        let mut fix: FbFixScreeninfo = unsafe { std::mem::zeroed() };
        unsafe {
            assert!(libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut var) == 0, "FBIOGET_VSCREENINFO");
            assert!(libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut fix) == 0, "FBIOGET_FSCREENINFO");
        }
        println!(
            "fb0: {}x{} virtual {}x{} bpp {} stride {} smem {} yoffset {} r@{} g@{} b@{} a@{}",
            var.xres, var.yres, var.xres_virtual, var.yres_virtual, var.bits_per_pixel, fix.line_length,
            fix.smem_len, var.yoffset, var.red.offset, var.green.offset, var.blue.offset, var.transp.offset
        );
        let len = fix.smem_len as usize;
        let mem = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        };
        assert!(mem != libc::MAP_FAILED, "mmap fb0");
        Fb { file, mem: mem as *mut u8, len, var, stride: fix.line_length as usize }
    }

    /// Physical pixel write (panel coordinates, portrait).
    fn put_phys(&self, x: u32, y: u32, argb: u32) {
        if x >= self.var.xres || y >= self.var.yres {
            return;
        }
        let off = (y as usize + self.var.yoffset as usize) * self.stride + x as usize * 4;
        if off + 4 <= self.len {
            unsafe { std::ptr::write_volatile(self.mem.add(off) as *mut u32, argb) };
        }
    }

    fn pan(&self) {
        let mut v = self.var;
        v.activate = 0; // FB_ACTIVATE_NOW
        unsafe {
            libc::ioctl(self.file.as_raw_fd(), FBIOPAN_DISPLAY as _, &v);
        }
    }
}

/// Logical landscape canvas (960x540) mapped onto the portrait panel.
struct Canvas<'a> {
    fb: &'a Fb,
    w: u32,
    h: u32,
    rot: u32,
}

impl<'a> Canvas<'a> {
    fn put(&self, lx: i32, ly: i32, argb: u32) {
        if lx < 0 || ly < 0 || lx as u32 >= self.w || ly as u32 >= self.h {
            return;
        }
        let (lx, ly) = (lx as u32, ly as u32);
        let (px, py) = match self.rot {
            0 => (lx, ly),
            1 => (self.h - 1 - ly, lx),
            2 => (self.w - 1 - lx, self.h - 1 - ly),
            _ => (ly, self.w - 1 - lx),
        };
        self.fb.put_phys(px, py, argb);
    }
    fn rect(&self, x: i32, y: i32, w: i32, h: i32, argb: u32) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, argb);
            }
        }
    }
}

/// Key slot geometry on the 960x540 canvas: 5 columns x 3 rows, wide slot at col 3-4 row 2.
fn slot_rect(index: usize) -> (i32, i32, i32, i32) {
    let cell_w = 960 / 5;
    let cell_h = 540 / 3;
    let pad = 12;
    if index == 13 {
        return (3 * cell_w + pad, 2 * cell_h + pad, 2 * cell_w - 2 * pad, cell_h - 2 * pad);
    }
    let col = (index % 5) as i32;
    let row = (index / 5) as i32;
    (col * cell_w + pad, row * cell_h + pad, cell_w - 2 * pad, cell_h - 2 * pad)
}

struct State {
    matrix: u16,
    discrete: u8,
    knob_pos: [i32; 3],
}

fn draw(c: &Canvas, s: &State) {
    c.rect(0, 0, 960, 540, 0xff202020);
    for i in 0..14 {
        if i == 13 {
            continue;
        }
        let (x, y, w, h) = slot_rect(i);
        let pressed = s.matrix & (0x4000 >> i) != 0;
        let color = if pressed { 0xffffa000 } else { 0xff404850 };
        c.rect(x, y, w, h, color);
    }
    // wide slot: knob bars
    let (x, y, w, h) = slot_rect(13);
    let wide_pressed = s.matrix & 0x0001 != 0;
    c.rect(x, y, w, h, if wide_pressed { 0xffffa000 } else { 0xff304050 });
    for k in 0..3 {
        let bar_h = (h - 16) / 3;
        let by = y + 8 + k as i32 * bar_h;
        let pressed = s.discrete & (0x04 << k) != 0;
        let base = if pressed { 0xffff4040 } else { 0xff80c0ff };
        let len = ((s.knob_pos[k].rem_euclid(40)) * (w - 16) / 40).max(2);
        c.rect(x + 8, by + 2, len, bar_h - 4, base);
    }
    // side buttons: thin bars at the far left/right edges of row 2 col 0 area
    if s.discrete & 0x01 != 0 {
        c.rect(0, 0, 8, 540, 0xff00ff80);
    }
    if s.discrete & 0x02 != 0 {
        c.rect(952, 0, 8, 540, 0xff00ff80);
    }
    // orientation markers
    c.rect(0, 0, 24, 24, 0xffff0000); // red top-left
    c.rect(936, 0, 24, 24, 0xff00ff00); // green top-right
    c.rect(0, 516, 24, 24, 0xff0000ff); // blue bottom-left
    c.fb.pan();
}

fn open_uart() -> std::fs::File {
    let port = std::fs::OpenOptions::new().read(true).write(true).open("/dev/ttyS1").expect("open ttyS1");
    let fd = port.as_raw_fd();
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        libc::tcgetattr(fd, &mut t);
        libc::cfmakeraw(&mut t);
        t.c_cflag |= libc::CLOCAL | libc::CREAD;
        t.c_cflag &= !(libc::PARENB | libc::CSTOPB | libc::CRTSCTS);
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 1;
        libc::cfsetispeed(&mut t, libc::B115200);
        libc::cfsetospeed(&mut t, libc::B115200);
        libc::tcsetattr(fd, libc::TCSANOW, &t);
        libc::tcflush(fd, libc::TCIOFLUSH);
    }
    port
}

fn knob_dir(nibble: u8) -> i32 {
    match nibble {
        0x01 | 0x07 | 0x0e | 0x08 => 1,
        0x0d | 0x04 | 0x02 | 0x0b => -1,
        _ => 0,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    // Verified on hardware 2026-10-29: rotation 3 puts logical top-left at the
    // deck's top-left (rotation 1 came out upside down).
    let rot: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
    let fb = Fb::open();
    let canvas = Canvas { fb: &fb, w: 960, h: 540, rot };
    let mut uart = open_uart();
    let mut st = State { matrix: 0, discrete: 0, knob_pos: [0; 3] };
    draw(&canvas, &st);
    println!("drawn test pattern (rotation {rot}); reading MCU for {secs}s");
    let start = Instant::now();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 256];
    let mut dirty = false;
    while start.elapsed() < Duration::from_secs(secs) {
        if let Ok(n) = uart.read(&mut tmp) {
            buf.extend_from_slice(&tmp[..n]);
        }
        // parse frames
        loop {
            if buf.len() < 7 {
                break;
            }
            if buf[0] != 0xff || buf[1] != 0x55 {
                buf.remove(0);
                continue;
            }
            let cmd = buf[2] as u16 | ((buf[3] as u16) << 8);
            let len = buf[4] as usize;
            let total = 5 + len + 2;
            if buf.len() < total {
                break;
            }
            let sum: u16 = buf[..total - 2].iter().map(|&b| b as u16).fold(0, |a, b| a.wrapping_add(b));
            let cs = ((buf[total - 2] as u16) << 8) | buf[total - 1] as u16;
            let ms = start.elapsed().as_millis();
            if sum != cs {
                println!("{ms:7} ms bad checksum");
                buf.drain(..2);
                continue;
            }
            let payload = buf[5..5 + len].to_vec();
            buf.drain(..total);
            match (cmd, len) {
                (0x0001, 3) => {
                    st.discrete = payload[0];
                    st.matrix = ((payload[1] as u16) << 8) | payload[2] as u16;
                    let keys: Vec<usize> = (0..14).filter(|&i| st.matrix & (if i == 13 { 1 } else { 0x4000 >> i }) != 0).collect();
                    println!("{ms:7} ms keys {:?} discrete 0x{:02x}", keys, st.discrete);
                    dirty = true;
                }
                (0x0002..=0x0004, 1) => {
                    let k = (cmd - 2) as usize;
                    let d = knob_dir(payload[0]);
                    st.knob_pos[k] += d;
                    println!("{ms:7} ms knob{} step {:+} (nibble 0x{:02x}) pos {}", k + 1, d, payload[0], st.knob_pos[k]);
                    dirty = true;
                }
                _ => println!("{ms:7} ms cmd 0x{cmd:04x} {:02x?}", payload),
            }
        }
        if dirty {
            draw(&canvas, &st);
            dirty = false;
        }
    }
    // clear to black on exit
    canvas.rect(0, 0, 960, 540, 0xff000000);
    fb.pan();
    let _ = std::io::stdout().flush();
    println!("done");
}
