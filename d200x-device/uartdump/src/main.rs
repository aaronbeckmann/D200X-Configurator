//! uartdump: open a serial port raw at a given baud rate, read for N seconds,
//! write a hex dump with timestamps to stdout and the raw bytes to a file.
//!
//! Usage: uartdump <dev> <baud> <seconds> <rawfile>
//! e.g.   uartdump /dev/ttyS1 115200 60 /tmp/uart.bin
//!
//! Static armv7 musl build; runs from /tmp on the Ulanzi D200X (SSD210).

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

fn baud_const(b: u32) -> Option<libc::speed_t> {
    Some(match b {
        9600 => libc::B9600,
        19200 => libc::B19200,
        38400 => libc::B38400,
        57600 => libc::B57600,
        115200 => libc::B115200,
        230400 => libc::B230400,
        460800 => libc::B460800,
        500000 => libc::B500000,
        921600 => libc::B921600,
        1000000 => libc::B1000000,
        1500000 => libc::B1500000,
        _ => return None,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 && args.len() != 6 {
        eprintln!("usage: uartdump <dev> <baud> <seconds> <rawfile> [hex-bytes-to-send-first]");
        std::process::exit(2);
    }
    let tx: Vec<u8> = if args.len() == 6 {
        let h = args[5].replace(' ', "");
        (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).expect("hex")).collect()
    } else {
        Vec::new()
    };
    let dev = &args[1];
    let baud: u32 = args[2].parse().expect("baud");
    let secs: u64 = args[3].parse().expect("seconds");
    let rawpath = &args[4];

    let mut port = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dev)
        .unwrap_or_else(|e| {
            eprintln!("open {dev}: {e}");
            std::process::exit(1)
        });
    let fd = port.as_raw_fd();
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) != 0 {
            eprintln!("tcgetattr failed");
            std::process::exit(1);
        }
        libc::cfmakeraw(&mut t);
        t.c_cflag |= libc::CLOCAL | libc::CREAD;
        t.c_cflag &= !(libc::PARENB | libc::CSTOPB | libc::CRTSCTS);
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 1; // 100 ms read timeout
        if let Some(sp) = baud_const(baud) {
            libc::cfsetispeed(&mut t, sp);
            libc::cfsetospeed(&mut t, sp);
        } else {
            eprintln!("unsupported baud {baud}");
            std::process::exit(1);
        }
        if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
            eprintln!("tcsetattr failed");
            std::process::exit(1);
        }
        libc::tcflush(fd, libc::TCIOFLUSH);
    }

    let mut raw = File::create(rawpath).expect("create raw file");
    let start = Instant::now();
    let mut buf = [0u8; 256];
    let mut total = 0usize;
    println!("uartdump {dev} @ {baud} for {secs}s");
    if !tx.is_empty() {
        let hex: Vec<String> = tx.iter().map(|b| format!("{b:02x}")).collect();
        println!("     tx  {}", hex.join(" "));
        port.write_all(&tx).ok();
    }
    while start.elapsed() < Duration::from_secs(secs) {
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                total += n;
                raw.write_all(&buf[..n]).ok();
                let ms = start.elapsed().as_millis();
                let hex: Vec<String> = buf[..n].iter().map(|b| format!("{b:02x}")).collect();
                println!("{ms:7} ms  {}", hex.join(" "));
            }
            Err(e) => {
                eprintln!("read error: {e}");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    println!("done, {total} bytes");
}
