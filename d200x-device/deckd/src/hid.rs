//! USB gadget: keyboard reports on /dev/hidg1, host config channel on
//! /dev/hidg0 (1024-byte reports, stock `7c 7c` framing).

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::Arc;
use std::time::Duration;

pub const REPORT_LEN: usize = 1024;

pub fn switch_gadget(mode: &str) {
    let _ = Command::new("/bin/setprop").args(["sys.usb.config", mode]).status();
}

pub fn setprop(name: &str, value: &str) {
    let _ = Command::new("/bin/setprop").args([name, value]).status();
}

pub struct Keyboard {
    dev: Option<File>,
}

impl Keyboard {
    pub fn closed() -> Keyboard {
        Keyboard { dev: None }
    }
    pub fn open() -> Keyboard {
        let dev = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/hidg1")
            .ok();
        Keyboard { dev }
    }
    pub fn is_open(&self) -> bool {
        self.dev.is_some()
    }
    pub fn report(&mut self, modifiers: u8, keys: &[u8]) -> bool {
        let mut r = [0u8; 8];
        r[0] = modifiers;
        for (i, k) in keys.iter().take(6).enumerate() {
            r[2 + i] = *k;
        }
        match self.dev.as_mut() {
            Some(d) => d.write(&r).is_ok(),
            None => false,
        }
    }
    pub fn tap(&mut self, modifiers: u8, key: u8) {
        self.report(modifiers, &[key]);
        std::thread::sleep(Duration::from_millis(12));
        self.report(0, &[]);
        std::thread::sleep(Duration::from_millis(8));
    }
    /// Type an ASCII string with a US layout.
    pub fn type_text(&mut self, s: &str) {
        for ch in s.chars() {
            if let Some((m, k)) = ascii_to_usage(ch) {
                self.tap(m, k);
            }
        }
    }
}

enum KeyJob {
    Chord(u8, Vec<u8>),
    Text(String),
}

/// Keyboard output on its own thread: a key report has to be held for a while and typing takes
/// as long as it takes, neither may stop the deck from reacting.
pub struct KeyQueue {
    jobs: Sender<KeyJob>,
    waiting: Arc<AtomicUsize>,
}

impl KeyQueue {
    pub fn start(mut kbd: Keyboard) -> KeyQueue {
        let (jobs, rx) = mpsc::channel();
        let waiting = Arc::new(AtomicUsize::new(0));
        let left = waiting.clone();
        std::thread::spawn(move || {
            for job in rx {
                match job {
                    KeyJob::Chord(mods, usage) => {
                        kbd.report(mods, &usage);
                        std::thread::sleep(Duration::from_millis(12));
                        kbd.report(0, &[]);
                    }
                    KeyJob::Text(text) => kbd.type_text(&text),
                }
                left.fetch_sub(1, Ordering::SeqCst);
            }
        });
        KeyQueue { jobs, waiting }
    }
    fn push(&self, job: KeyJob) {
        self.waiting.fetch_add(1, Ordering::SeqCst);
        let _ = self.jobs.send(job);
    }
    /// Press and release a key combination (`usage` empty: modifiers alone).
    pub fn chord(&self, mods: u8, usage: Vec<u8>) {
        self.push(KeyJob::Chord(mods, usage));
    }
    pub fn text(&self, text: &str) {
        self.push(KeyJob::Text(text.to_string()));
    }
    /// Is something still being sent?
    pub fn busy(&self) -> bool {
        self.waiting.load(Ordering::SeqCst) > 0
    }
}

pub const MOD_LCTRL: u8 = 0x01;
pub const MOD_LSHIFT: u8 = 0x02;
pub const MOD_LALT: u8 = 0x04;
pub const MOD_LGUI: u8 = 0x08;

pub fn ascii_to_usage(ch: char) -> Option<(u8, u8)> {
    let c = ch as u32;
    Some(match ch {
        'a'..='z' => (0, 0x04 + (c - 'a' as u32) as u8),
        'A'..='Z' => (MOD_LSHIFT, 0x04 + (c - 'A' as u32) as u8),
        '1'..='9' => (0, 0x1e + (c - '1' as u32) as u8),
        '0' => (0, 0x27),
        '\n' => (0, 0x28),
        '\t' => (0, 0x2b),
        ' ' => (0, 0x2c),
        '-' => (0, 0x2d),
        '=' => (0, 0x2e),
        '[' => (0, 0x2f),
        ']' => (0, 0x30),
        '\\' => (0, 0x31),
        ';' => (0, 0x33),
        '\'' => (0, 0x34),
        '`' => (0, 0x35),
        ',' => (0, 0x36),
        '.' => (0, 0x37),
        '/' => (0, 0x38),
        '!' => (MOD_LSHIFT, 0x1e),
        '@' => (MOD_LSHIFT, 0x1f),
        '#' => (MOD_LSHIFT, 0x20),
        '$' => (MOD_LSHIFT, 0x21),
        '%' => (MOD_LSHIFT, 0x22),
        '^' => (MOD_LSHIFT, 0x23),
        '&' => (MOD_LSHIFT, 0x24),
        '*' => (MOD_LSHIFT, 0x25),
        '(' => (MOD_LSHIFT, 0x26),
        ')' => (MOD_LSHIFT, 0x27),
        '_' => (MOD_LSHIFT, 0x2d),
        '+' => (MOD_LSHIFT, 0x2e),
        '{' => (MOD_LSHIFT, 0x2f),
        '}' => (MOD_LSHIFT, 0x30),
        '|' => (MOD_LSHIFT, 0x31),
        ':' => (MOD_LSHIFT, 0x33),
        '"' => (MOD_LSHIFT, 0x34),
        '~' => (MOD_LSHIFT, 0x35),
        '<' => (MOD_LSHIFT, 0x36),
        '>' => (MOD_LSHIFT, 0x37),
        '?' => (MOD_LSHIFT, 0x38),
        _ => return None,
    })
}

/// Key names accepted in config: letters, digits, f1..f24, named keys.
pub fn key_usage(name: &str) -> Option<u8> {
    let n = name.to_ascii_lowercase();
    if n.len() == 1 {
        return ascii_to_usage(n.chars().next().unwrap()).map(|(_, k)| k);
    }
    if let Some(f) = n.strip_prefix('f') {
        if let Ok(num) = f.parse::<u8>() {
            return match num {
                1..=12 => Some(0x3a + num - 1),
                13..=24 => Some(0x68 + num - 13),
                _ => None,
            };
        }
    }
    Some(match n.as_str() {
        "enter" | "return" => 0x28,
        "esc" | "escape" => 0x29,
        "backspace" => 0x2a,
        "tab" => 0x2b,
        "space" => 0x2c,
        "capslock" => 0x39,
        "printscreen" => 0x46,
        "scrolllock" => 0x47,
        "pause" => 0x48,
        "insert" => 0x49,
        "home" => 0x4a,
        "pageup" => 0x4b,
        "delete" => 0x4c,
        "end" => 0x4d,
        "pagedown" => 0x4e,
        "right" => 0x4f,
        "left" => 0x50,
        "down" => 0x51,
        "up" => 0x52,
        "numlock" => 0x53,
        "menu" => 0x65,
        "mute" => 0x7f,
        "volumeup" => 0x80,
        "volumedown" => 0x81,
        "numpad/" | "kpdivide" => 0x54,
        "numpad*" | "kpmultiply" => 0x55,
        "numpad-" | "kpminus" => 0x56,
        "numpad+" | "kpplus" => 0x57,
        "numpadenter" | "kpenter" => 0x58,
        "numpad1" | "kp1" => 0x59,
        "numpad2" | "kp2" => 0x5a,
        "numpad3" | "kp3" => 0x5b,
        "numpad4" | "kp4" => 0x5c,
        "numpad5" | "kp5" => 0x5d,
        "numpad6" | "kp6" => 0x5e,
        "numpad7" | "kp7" => 0x5f,
        "numpad8" | "kp8" => 0x60,
        "numpad9" | "kp9" => 0x61,
        "numpad0" | "kp0" => 0x62,
        "numpad." | "kpdot" => 0x63,
        _ => return None,
    })
}

pub fn modifier_bit(name: &str) -> Option<u8> {
    Some(match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" | "lctrl" => MOD_LCTRL,
        "shift" | "lshift" => MOD_LSHIFT,
        "alt" | "lalt" | "option" => MOD_LALT,
        "gui" | "win" | "cmd" | "super" | "meta" | "lgui" => MOD_LGUI,
        "rctrl" => 0x10,
        "rshift" => 0x20,
        "ralt" | "altgr" => 0x40,
        "rgui" => 0x80,
        _ => return None,
    })
}

/// Vendor channel (interface 0): 1024-byte reports both ways.
pub struct Vendor {
    dev: Option<File>,
    /// Outgoing messages: a host that does not read must not stall the deck.
    tx: Option<SyncSender<(u16, Vec<u8>)>>,
    rx: Vec<u8>,
    rx_need: usize,
    rx_cmd: u16,
}

pub struct Message {
    pub cmd: u16,
    pub payload: Vec<u8>,
}

impl Vendor {
    pub fn closed() -> Vendor {
        Vendor { dev: None, tx: None, rx: Vec::new(), rx_need: 0, rx_cmd: 0 }
    }
    pub fn open() -> Vendor {
        let dev = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/hidg0")
            .ok();
        let tx = dev.as_ref().and_then(|d| d.try_clone().ok()).map(|mut out| {
            let (tx, rx) = mpsc::sync_channel::<(u16, Vec<u8>)>(32);
            std::thread::spawn(move || {
                for (cmd, payload) in rx {
                    send_message(&mut out, cmd, &payload);
                }
            });
            tx
        });
        Vendor { dev, tx, rx: Vec::new(), rx_need: 0, rx_cmd: 0 }
    }
    pub fn is_open(&self) -> bool {
        self.dev.is_some()
    }
    pub fn fd(&self) -> Option<RawFd> {
        self.dev.as_ref().map(|d| d.as_raw_fd())
    }

    /// Read available reports; return complete messages.
    pub fn poll(&mut self) -> Vec<Message> {
        let mut out = Vec::new();
        let Some(dev) = self.dev.as_mut() else { return out };
        let mut buf = [0u8; REPORT_LEN];
        loop {
            let n = match dev.read(&mut buf) {
                Ok(n) if n > 0 => n,
                _ => break,
            };
            let data = &buf[..n];
            if self.rx_need == 0 {
                if n >= 8 && data[0] == 0x7c && data[1] == 0x7c {
                    self.rx_cmd = u16::from_be_bytes([data[2], data[3]]);
                    self.rx_need = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
                    self.rx.clear();
                    let take = self.rx_need.min(n - 8);
                    self.rx.extend_from_slice(&data[8..8 + take]);
                } else {
                    continue;
                }
            } else {
                let take = (self.rx_need - self.rx.len()).min(n);
                self.rx.extend_from_slice(&data[..take]);
            }
            if self.rx.len() >= self.rx_need {
                out.push(Message { cmd: self.rx_cmd, payload: std::mem::take(&mut self.rx) });
                self.rx_need = 0;
            }
        }
        out
    }

    /// Queue a message for the host; dropped while the host has stopped reading.
    pub fn send(&mut self, cmd: u16, payload: &[u8]) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send((cmd, payload.to_vec()));
        }
    }
}

/// Send a message with the stock framing (one or more 1024-byte reports).
fn send_message(dev: &mut File, cmd: u16, payload: &[u8]) -> bool {
    let mut first = [0u8; REPORT_LEN];
    first[0] = 0x7c;
    first[1] = 0x7c;
    first[2..4].copy_from_slice(&cmd.to_be_bytes());
    first[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    let n = payload.len().min(REPORT_LEN - 8);
    first[8..8 + n].copy_from_slice(&payload[..n]);
    if write_full(dev, &first).is_err() {
        return false;
    }
    for chunk in payload[n..].chunks(REPORT_LEN) {
        let mut r = [0u8; REPORT_LEN];
        r[..chunk.len()].copy_from_slice(chunk);
        if write_full(dev, &r).is_err() {
            return false;
        }
    }
    true
}

fn write_full(dev: &mut File, r: &[u8]) -> std::io::Result<()> {
    // Non-blocking endpoint: retry briefly while the host drains.
    for _ in 0..200 {
        match dev.write(r) {
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "hidg0 write timeout"))
}
