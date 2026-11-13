//! Device serial number from the SPI NOR OTP "security register" page 3,
//! read the way the vendor app does: libeasyui `zk_security_read_data` ->
//! oflash backend -> ioctl(fd("/dev/oflash"), 0x80086f04, &{page, len, _, buf}).
//! (Recovered by disassembly, see research/INVESTIGATION.md.)

use std::os::unix::io::AsRawFd;

const OFLASH: &str = "/dev/oflash";
const IOCTL_READ_SEC: libc::c_ulong = 0x80086f04;
/// Authentication issued by the vendor's open helper right after open():
/// ioctl(fd, _IOW('o', 3, u32), &0x1d2f2736). Without it the read returns EPERM.
const IOCTL_SETUP: libc::c_ulong = 0x40046f03;
const SETUP_ARG: [u8; 4] = [0x36, 0x27, 0x2f, 0x1d];

#[repr(C)]
struct SecReq {
    page: u8,
    len: u8,
    _pad: u16,
    buf: u32,
}

pub fn read_sec_page_raw(page: u8, len: usize, setup: bool) -> Result<Vec<u8>, String> {
    let len = len.min(0x100);
    let f = std::fs::OpenOptions::new().read(true).write(true).open(OFLASH).map_err(|e| format!("{OFLASH}: {e}"))?;
    if setup {
        let r = unsafe { libc::ioctl(f.as_raw_fd(), IOCTL_SETUP as _, SETUP_ARG.as_ptr()) };
        if r != 0 {
            return Err(format!("ioctl setup: {}", std::io::Error::last_os_error()));
        }
    }
    let mut buf = vec![0u8; 0x100];
    let req = SecReq { page, len: len as u8, _pad: 0, buf: buf.as_mut_ptr() as usize as u32 };
    let r = unsafe { libc::ioctl(f.as_raw_fd(), IOCTL_READ_SEC as _, &req as *const SecReq) };
    if r < 0 {
        return Err(format!("ioctl read_sec page {page}: {}", std::io::Error::last_os_error()));
    }
    buf.truncate(len);
    Ok(buf)
}

pub fn read_sec_page(page: u8, len: usize) -> Result<Vec<u8>, String> {
    read_sec_page_raw(page, len, false)
}

fn plausible(b: &[u8]) -> Option<String> {
    let end = b.iter().position(|&c| c == 0 || c == 0xff).unwrap_or(b.len());
    let s: String = b[..end].iter().map(|&c| c as char).collect();
    if s.len() >= 8 && s.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(s)
    } else {
        None
    }
}

/// The 17-character serial the stock firmware reports (e.g. 02D23A045U3672338).
pub fn serial_number() -> Option<String> {
    if let Ok(b) = read_sec_page_raw(3, 17, true) {
        if let Some(s) = plausible(&b) {
            return Some(s);
        }
    }
    read_sec_page_raw(3, 17, false).ok().and_then(|b| plausible(&b))
}
