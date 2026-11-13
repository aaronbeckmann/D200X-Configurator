//! hidkeys: standalone demo. Reads the MCU on /dev/ttyS1, switches the USB
//! gadget to the stock "hid" configuration (keyboard on /dev/hidg0, vendor
//! channel on /dev/hidg1) exactly as the vendor app does, and sends boot
//! keyboard reports for every key:
//!   key 0..12 -> letters a..m, wide key -> Enter, side buttons -> Page Up/Down,
//!   knob press -> Space, knob turn -> Left/Right arrow.
//! Draws a simple key grid on /dev/fb0 so the deck shows it is alive.
//! Exits by rebooting the deck after <seconds>. Log goes to /data/hidkeys.log.
//!
//! Usage: hidkeys <seconds>

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::process::Command;
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
    file: File,
    mem: *mut u8,
    len: usize,
    var: FbVarScreeninfo,
    stride: usize,
}

impl Fb {
    fn open() -> Option<Fb> {
        let file = std::fs::OpenOptions::new().read(true).write(true).open("/dev/fb0").ok()?;
        let fd = file.as_raw_fd();
        let mut var = FbVarScreeninfo::default();
        let mut fix: FbFixScreeninfo = unsafe { std::mem::zeroed() };
        unsafe {
            if libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut var) != 0 {
                return None;
            }
            if libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut fix) != 0 {
                return None;
            }
        }
        let len = fix.smem_len as usize;
        let mem = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        };
        if mem == libc::MAP_FAILED {
            return None;
        }
        Some(Fb { file, mem: mem as *mut u8, len, var, stride: fix.line_length as usize })
    }
    /// Logical 960x540 landscape pixel -> physical (rotation 3, verified).
    fn put(&self, lx: i32, ly: i32, argb: u32) {
        if lx < 0 || ly < 0 || lx >= 960 || ly >= 540 {
            return;
        }
        let (px, py) = (ly as usize, (959 - lx) as usize);
        let off = (py + self.var.yoffset as usize) * self.stride + px * 4;
        if off + 4 <= self.len {
            unsafe { std::ptr::write_volatile(self.mem.add(off) as *mut u32, argb) };
        }
    }
    fn rect(&self, x: i32, y: i32, w: i32, h: i32, argb: u32) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, argb);
            }
        }
    }
    fn pan(&self) {
        let v = self.var;
        unsafe {
            libc::ioctl(self.file.as_raw_fd(), FBIOPAN_DISPLAY as _, &v);
        }
    }
}

fn slot_rect(index: usize) -> (i32, i32, i32, i32) {
    let (cw, ch, pad) = (960 / 5, 540 / 3, 12);
    if index == 13 {
        return (3 * cw + pad, 2 * ch + pad, 2 * cw - 2 * pad, ch - 2 * pad);
    }
    let (col, row) = ((index % 5) as i32, (index / 5) as i32);
    (col * cw + pad, row * ch + pad, cw - 2 * pad, ch - 2 * pad)
}

fn draw(fb: &Option<Fb>, matrix: u16, discrete: u8, usb_ready: bool) {
    let Some(fb) = fb else { return };
    fb.rect(0, 0, 960, 540, if usb_ready { 0xff102030 } else { 0xff301010 });
    for i in 0..14 {
        let (x, y, w, h) = slot_rect(i);
        let bit = if i == 13 { 1 } else { 0x4000 >> i };
        fb.rect(x, y, w, h, if matrix & bit != 0 { 0xffffa000 } else { 0xff405060 });
    }
    if discrete & 0x01 != 0 {
        fb.rect(0, 0, 8, 540, 0xff00ff80);
    }
    if discrete & 0x02 != 0 {
        fb.rect(952, 0, 8, 540, 0xff00ff80);
    }
    fb.pan();
}

fn open_uart() -> File {
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

struct Kbd {
    dev: Option<File>,
    log: File,
}

impl Kbd {
    fn open(path: &str, log: File) -> Kbd {
        let dev = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .ok();
        Kbd { dev, log }
    }
    fn report(&mut self, modifiers: u8, key: u8) {
        let r = [modifiers, 0, key, 0, 0, 0, 0, 0];
        match self.dev.as_mut() {
            Some(d) => match d.write(&r) {
                Ok(n) => {
                    let _ = writeln!(self.log, "hid report {:02x?} -> {n}", r);
                }
                Err(e) => {
                    let _ = writeln!(self.log, "hid report {:02x?} failed: {e}", r);
                }
            },
            None => {
                let _ = writeln!(self.log, "hidg0 not open");
            }
        }
    }
    fn tap(&mut self, key: u8) {
        self.report(0, key);
        std::thread::sleep(Duration::from_millis(15));
        self.report(0, 0);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    // USB interface 0 is the 1024-byte vendor collection and interface 1 the
    // boot keyboard (host-side descriptors, 2026-10-29), so the keyboard is
    // expected on the second gadget node.
    let kbd_path = args.get(2).cloned().unwrap_or_else(|| "/dev/hidg1".to_string());
    let mut log = File::create("/data/hidkeys.log").expect("log");
    let _ = writeln!(log, "hidkeys start, {secs}s, keyboard node {kbd_path}");

    let fb = Fb::open();
    let mut uart = open_uart();
    let (mut matrix, mut discrete) = (0u16, 0u8);
    draw(&fb, 0, 0, false);

    // Switch the gadget to the stock HID configuration (what zkgui does).
    let st = Command::new("/bin/setprop").args(["sys.usb.config", "hid"]).status();
    let _ = writeln!(log, "setprop sys.usb.config hid -> {:?}", st);
    std::thread::sleep(Duration::from_secs(3));
    let mut kbd = Kbd::open(&kbd_path, log.try_clone().unwrap());
    let _ = writeln!(log, "{kbd_path} open: {}", kbd.dev.is_some());
    draw(&fb, 0, 0, kbd.dev.is_some());

    let start = Instant::now();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 256];
    while start.elapsed() < Duration::from_secs(secs) {
        if let Ok(n) = uart.read(&mut tmp) {
            buf.extend_from_slice(&tmp[..n]);
        }
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
            if sum != cs {
                buf.drain(..2);
                continue;
            }
            let payload = buf[5..5 + len].to_vec();
            buf.drain(..total);
            match (cmd, len) {
                (0x0001, 3) => {
                    let new_d = payload[0];
                    let new_m = ((payload[1] as u16) << 8) | payload[2] as u16;
                    let pressed_m = new_m & !matrix;
                    let pressed_d = new_d & !discrete;
                    matrix = new_m;
                    discrete = new_d;
                    let _ = writeln!(log, "{} ms state m=0x{matrix:04x} d=0x{discrete:02x}", start.elapsed().as_millis());
                    for i in 0..13 {
                        if pressed_m & (0x4000 >> i) != 0 {
                            kbd.tap(0x04 + i as u8); // a..m
                        }
                    }
                    if pressed_m & 1 != 0 {
                        kbd.tap(0x28); // Enter
                    }
                    if pressed_d & 0x01 != 0 {
                        kbd.tap(0x4b); // Page Up
                    }
                    if pressed_d & 0x02 != 0 {
                        kbd.tap(0x4e); // Page Down
                    }
                    if pressed_d & 0x1c != 0 {
                        kbd.tap(0x2c); // Space for any knob press
                    }
                    draw(&fb, matrix, discrete, kbd.dev.is_some());
                }
                (0x0002..=0x0004, 1) => {
                    // one keystroke per detent (4 transitions)
                    static mut ACC: [i32; 3] = [0; 3];
                    let k = (cmd - 2) as usize;
                    let d = knob_dir(payload[0]);
                    let fire = unsafe {
                        ACC[k] += d;
                        if ACC[k] >= 4 {
                            ACC[k] = 0;
                            1
                        } else if ACC[k] <= -4 {
                            ACC[k] = 0;
                            -1
                        } else {
                            0
                        }
                    };
                    if fire == 1 {
                        kbd.tap(0x4f); // Right arrow
                    } else if fire == -1 {
                        kbd.tap(0x50); // Left arrow
                    }
                }
                _ => {}
            }
        }
    }
    let _ = writeln!(log, "done, rebooting");
    if let Some(fb) = &fb {
        fb.rect(0, 0, 960, 540, 0xff000000);
        fb.pan();
    }
    let _ = Command::new("/bin/reboot").status();
}
