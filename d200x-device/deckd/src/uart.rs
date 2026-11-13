//! Agent link on the SoC console UART /dev/ttyS0: the ESP32 bridge (esp32-bridge/docs/INTERFACE.md).
//! The console normally carries a root shell (`service console /bin/sh` in init.rc); it is stopped
//! before the port is opened, because every byte the bridge sends would otherwise be shell input.

use crate::frame::{self, Parser};
use crate::hid::{self, Message};
use crate::Log;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::mpsc::{self, SyncSender};
use std::time::{Duration, Instant};

const PORT: &str = "/dev/ttyS0";

pub struct SerialLink {
    port: Option<File>,
    tx: Option<SyncSender<Vec<u8>>>,
    parser: Parser,
    /// Frames not sent because the writer was behind.
    pub dropped: u64,
}

impl SerialLink {
    pub fn closed() -> SerialLink {
        SerialLink { port: None, tx: None, parser: Parser::default(), dropped: 0 }
    }

    pub fn open(baud: u32, log: &Log) -> SerialLink {
        stop_console(log);
        let port = match std::fs::OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK).open(PORT) {
            Ok(p) => p,
            Err(e) => {
                log.line(&format!("uart: open {PORT}: {e}"));
                return SerialLink::closed();
            }
        };
        let fd = port.as_raw_fd();
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            libc::tcgetattr(fd, &mut t);
            libc::cfmakeraw(&mut t);
            t.c_cflag |= libc::CLOCAL | libc::CREAD;
            t.c_cflag &= !(libc::PARENB | libc::CSTOPB | libc::CRTSCTS);
            t.c_cc[libc::VMIN] = 0;
            t.c_cc[libc::VTIME] = 0;
            let speed = speed_of(baud);
            libc::cfsetispeed(&mut t, speed);
            libc::cfsetospeed(&mut t, speed);
            libc::tcsetattr(fd, libc::TCSANOW, &t);
            libc::ioctl(fd, libc::TIOCEXCL);
            libc::tcflush(fd, libc::TCIOFLUSH);
        }
        let tx = port.try_clone().ok().map(|mut out| {
            let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(32);
            std::thread::spawn(move || {
                for f in rx {
                    write_all(&mut out, &f);
                }
            });
            tx
        });
        log.line(&format!("uart: {PORT} open at {baud}"));
        SerialLink { port: Some(port), tx, parser: Parser::default(), dropped: 0 }
    }

    pub fn is_open(&self) -> bool {
        self.port.is_some()
    }

    pub fn fd(&self) -> Option<RawFd> {
        self.port.as_ref().map(|p| p.as_raw_fd())
    }

    /// Read what has arrived; return complete messages. Call every loop turn, also without input,
    /// so a stalled half frame is given up.
    pub fn poll(&mut self) -> Vec<Message> {
        let Some(port) = self.port.as_mut() else { return Vec::new() };
        let mut data = Vec::new();
        let mut buf = [0u8; 1024];
        while let Ok(n) = port.read(&mut buf) {
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
        }
        self.parser.push(&data, Instant::now()).into_iter().map(|(cmd, payload)| Message { cmd, payload }).collect()
    }

    pub fn errors(&self) -> u64 {
        self.parser.errors
    }

    /// Queue a frame; dropped while the writer is behind (the UART carries about 11 KB/s).
    pub fn send(&mut self, cmd: u16, payload: &[u8]) {
        if let Some(tx) = &self.tx {
            if tx.try_send(frame::encode(cmd, payload)).is_err() {
                self.dropped += 1;
            }
        }
    }
}

fn write_all(out: &mut File, mut f: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !f.is_empty() && Instant::now() < deadline {
        match out.write(f) {
            Ok(n) => f = &f[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(2)),
            Err(_) => return,
        }
    }
}

fn speed_of(baud: u32) -> libc::speed_t {
    match baud {
        230400 => libc::B230400,
        460800 => libc::B460800,
        921600 => libc::B921600,
        1500000 => libc::B1500000,
        _ => libc::B115200,
    }
}

/// Stop the console shell and kill any shell still holding the console. Returns how many were killed.
pub fn stop_console(log: &Log) -> usize {
    hid::setprop("ctl.stop", "console");
    let me = std::process::id();
    let mut killed = 0;
    let Ok(rd) = std::fs::read_dir("/proc") else { return 0 };
    for e in rd.filter_map(|e| e.ok()) {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        if pid <= 1 || pid == me {
            continue;
        }
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        if comm.trim() != "sh" {
            continue;
        }
        let holds = std::fs::read_dir(format!("/proc/{pid}/fd"))
            .map(|fds| {
                fds.filter_map(|f| f.ok())
                    .filter_map(|f| std::fs::read_link(f.path()).ok())
                    .any(|t| t.as_os_str() == "/dev/console" || t.as_os_str() == PORT)
            })
            .unwrap_or(false);
        if holds && unsafe { libc::kill(pid as i32, libc::SIGKILL) } == 0 {
            killed += 1;
            log.line(&format!("uart: killed console shell pid {pid}"));
        }
    }
    killed
}

/// Bytes the kernel has sent and received on the console port (for the log).
pub fn counters() -> String {
    // the driver's file name is vendor specific; the one listing port 0 is ours
    let Ok(rd) = std::fs::read_dir("/proc/tty/driver") else { return String::new() };
    for e in rd.filter_map(|e| e.ok()) {
        let mut s = String::new();
        if File::open(e.path()).and_then(|mut f| f.read_to_string(&mut s)).is_ok() {
            if let Some(l) = s.lines().find(|l| l.starts_with("0:")) {
                return l.to_string();
            }
        }
    }
    String::new()
}
