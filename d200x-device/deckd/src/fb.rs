//! Framebuffer output: the 960x540 canvas blitted onto the 540x960 portrait
//! panel with the verified rotation. Only what changed is copied.

use crate::canvas::{Canvas, Rect, H, W};
use std::fs::File;
use std::os::unix::io::AsRawFd;

/// What a frame changed on the canvas.
pub enum Dirty {
    None,
    Full,
    Rects(Vec<Rect>),
}

/// Side of the squares rotated at a time, through a temporary buffer that fits the cache: reading
/// the canvas column by column misses on every pixel (52 ms per screen on the SSD210, 11 ms like
/// this; 32 was the fastest of the sizes tried on the deck).
const BLOCK: usize = 32;

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

pub struct Fb {
    file: File,
    mem: *mut u8,
    len: usize,
    var: FbVarScreeninfo,
    stride: usize,
    page: u32,
    /// What the last frame changed: the back page is missing it too. None = everything.
    last: Option<Vec<Rect>>,
    tmp: Vec<u32>,
}

// The mapping is only used from the render thread.
unsafe impl Send for Fb {}

impl Fb {
    pub fn open() -> Result<Fb, String> {
        let file = std::fs::OpenOptions::new().read(true).write(true).open("/dev/fb0").map_err(|e| e.to_string())?;
        let fd = file.as_raw_fd();
        let mut var = FbVarScreeninfo::default();
        let mut fix: FbFixScreeninfo = unsafe { std::mem::zeroed() };
        unsafe {
            if libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut var) != 0 {
                return Err("FBIOGET_VSCREENINFO".into());
            }
            if libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut fix) != 0 {
                return Err("FBIOGET_FSCREENINFO".into());
            }
        }
        if var.xres != 540 || var.yres != 960 || var.bits_per_pixel != 32 {
            return Err(format!("unexpected fb geometry {}x{}x{}", var.xres, var.yres, var.bits_per_pixel));
        }
        let len = fix.smem_len as usize;
        let mem = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        };
        if mem == libc::MAP_FAILED {
            return Err("mmap".into());
        }
        Ok(Fb { file, mem: mem as *mut u8, len, var, stride: fix.line_length as usize, page: 0, last: None, tmp: vec![0; BLOCK * BLOCK] })
    }

    fn back_page(&self) -> u32 {
        if self.var.yres_virtual / self.var.yres > 1 {
            self.page ^ 1
        } else {
            0
        }
    }

    /// Blit what changed into the back page. False when there is nothing to show.
    pub fn blit(&mut self, c: &Canvas, dirty: &Dirty) -> bool {
        let full = [(0, 0, W as i32, H as i32)];
        let now: &[Rect] = match dirty {
            Dirty::None => return false,
            Dirty::Full => &full,
            Dirty::Rects(r) => r,
        };
        let base = self.back_page() as usize * self.var.yres as usize * self.stride;
        if base + self.var.yres as usize * self.stride > self.len {
            return false;
        }
        // the back page is two frames old: it lacks the last frame's changes as well as these
        let rects: Vec<Rect> = match self.last.take() {
            Some(last) if !matches!(dirty, Dirty::Full) => last.iter().chain(now).copied().collect(),
            _ => full.to_vec(),
        };
        for r in rects {
            self.blit_rect(c, r, base);
        }
        self.last = Some(now.to_vec());
        true
    }

    /// Rotation 3: physical (px, py) = (ly, 959 - lx), so canvas columns are physical rows.
    fn blit_rect(&mut self, c: &Canvas, r: Rect, base: usize) {
        let (x0, y0) = (r.0.max(0) as usize, r.1.max(0) as usize);
        let (x1, y1) = (((r.0 + r.2).max(0) as usize).min(W), ((r.1 + r.3).max(0) as usize).min(H));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (px, tmp) = (c.px.as_ptr(), self.tmp.as_mut_ptr());
        let mut lx = x0;
        while lx < x1 {
            let bw = BLOCK.min(x1 - lx);
            let mut ly = y0;
            while ly < y1 {
                let bh = BLOCK.min(y1 - ly);
                // Safety: (lx..lx + bw, ly..ly + bh) lies on the canvas, tmp holds BLOCK * BLOCK
                // and `blit` checked that the page lies in the mapping.
                unsafe {
                    for y in 0..bh {
                        let row = px.add((ly + y) * W + lx);
                        for i in 0..bw {
                            *tmp.add(i * bh + y) = *row.add(i);
                        }
                    }
                    for i in 0..bw {
                        let off = base + (W - 1 - lx - i) * self.stride + ly * 4;
                        std::ptr::copy_nonoverlapping(tmp.add(i * bh) as *const u8, self.mem.add(off), bh * 4);
                    }
                }
                ly += bh;
            }
            lx += bw;
        }
    }

    /// Show the back page. FBIOPAN_DISPLAY returns at the next vsync, so this paces the frames
    /// and must stay off the input path.
    pub fn flip(&mut self) {
        let page = self.back_page();
        let mut v = self.var;
        v.yoffset = page * self.var.yres;
        v.activate = 0;
        unsafe {
            libc::ioctl(self.file.as_raw_fd(), FBIOPAN_DISPLAY as _, &v);
        }
        self.page = page;
    }
}
