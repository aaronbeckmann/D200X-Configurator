


use anyhow::Result;
use clap::{Parser, Subcommand};
use d200x_core::Deck;
use d200x_core::*;

#[derive(Parser)]
#[command(name = "d200x-tool", about = "Ulanzi D200X host tool (vendor HID protocol)")]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List HID interfaces exposed by the deck
    List,
    /// Query serial number, firmware and hardware version
    Info,
    /// Dump the HID report descriptors of both interfaces (from the host side)
    Descriptors,
    /// Set key backlight brightness (0-100)
    Brightness { percent: u8 },
    /// deckd: push a config JSON file to the deck
    SetConfig { file: std::path::PathBuf },
    /// deckd: read the deck's config JSON (to stdout or a file)
    GetConfig { out: Option<std::path::PathBuf> },
    /// deckd: upload a PNG icon under a name
    SetIcon { name: String, file: std::path::PathBuf },
    /// deckd: reboot the deck
    Reboot,
    /// ESP32 bridge (via deckd): store its config JSON (Wi-Fi, connections, polls, subscriptions)
    BridgeConfig { file: std::path::PathBuf },
    /// ESP32 bridge (via deckd): print its status
    BridgeStatus,
    /// ESP32 bridge: install firmware over Wi-Fi. Serves the .bin from this PC and tells the
    /// bridge (via deckd) to download it; the bridge restarts into it and rolls back on failure.
    BridgeOta {
        /// Firmware image, e.g. esp32-bridge/build/esp32-bridge.bin
        file: std::path::PathBuf,
    },
    /// ESP32 bridge (via deckd): restart it
    BridgeReboot,
    /// ESP32 bridge (via deckd): every state its plugins know, one per line
    BridgeStates,
    /// deckd: print the tail of the device log
    Log,
    /// deckd: show (on) or hide (off) the tile-centering test pattern
    Calibrate { state: String },
    /// Print raw input reports (keys, side buttons, dials)
    Monitor {
        /// Seconds to run (0 = forever)
        #[arg(long, default_value_t = 0)]
        seconds: u64,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Command::List => {
            for l in Deck::list()? {
                println!("{l}");
            }
        }
        Command::Descriptors => {
            for (iface, desc) in Deck::descriptors()? {
                println!("interface {iface}:\n  {desc}");
            }
        }
        Command::Info => {
            let d = Deck::open()?;
            println!("{}", d.get_device_info()?);
        }
        Command::SetConfig { file } => {
            let json = std::fs::read(&file)?;
            serde_json::from_slice::<serde_json::Value>(&json).map_err(|e| anyhow::anyhow!("not valid JSON: {e}"))?;
            let d = Deck::open()?;
            println!("{}", d.set_config(&json)?);
        }
        Command::GetConfig { out } => {
            let d = Deck::open()?;
            let json = d.get_config()?;
            match out {
                Some(p) => {
                    std::fs::write(&p, &json)?;
                    println!("wrote {} bytes to {}", json.len(), p.display());
                }
                None => println!("{json}"),
            }
        }
        Command::SetIcon { name, file } => {
            let png = std::fs::read(&file)?;
            let d = Deck::open()?;
            println!("{}", d.set_icon(&name, &png)?);
        }
        Command::Calibrate { state } => {
            let on = matches!(state.to_ascii_lowercase().as_str(), "on" | "1" | "true");
            let d = Deck::open()?;
            println!("{}", d.calibrate(on)?);
        }
        Command::Log => {
            let d = Deck::open()?;
            println!("{}", d.get_log()?);
        }
        Command::Reboot => {
            Deck::open()?.reboot()?;
            println!("reboot sent");
        }
        Command::BridgeConfig { file } => {
            let json = std::fs::read_to_string(&file)?;
            serde_json::from_str::<serde_json::Value>(&json).map_err(|e| anyhow::anyhow!("not valid JSON: {e}"))?;
            println!("{}", Deck::open()?.bridge_config(&json)?);
        }
        Command::BridgeStatus => {
            println!("{}", Deck::open()?.bridge_status()?);
        }
        Command::BridgeReboot => {
            println!("{}", Deck::open()?.bridge_command(r#"{"cmd":"reboot"}"#)?);
        }
        Command::BridgeOta { file } => bridge_ota(&file)?,
        Command::BridgeStates => {
            let (plugins, states) = d200x_core::bridge::states(&Deck::open()?)?;
            println!("plugins: {}", plugins.join(", "));
            for (k, v) in states {
                println!("{k} = {v}");
            }
        }
        Command::Brightness { percent } => {
            let d = Deck::open()?;
            println!("opened {:?} serial {:?}", d.product, d.serial);
            d.set_brightness(percent)?;
            println!("brightness -> {percent}");
        }
        Command::Monitor { seconds } => {
            let d = Deck::open()?;
            println!("opened {:?} serial {:?}; waiting for input...", d.product, d.serial);
            let start = std::time::Instant::now();
            loop {
                if seconds > 0 && start.elapsed().as_secs() >= seconds {
                    break;
                }
                if let Some(r) = d.read_input(500)? {
                    let hex: Vec<String> = r.payload.iter().take(16).map(|b| format!("{b:02x}")).collect();
                    print!("cmd=0x{:04x} len={} payload[..16]={}", r.cmd, r.payload.len(), hex.join(" "));
                    if (r.cmd == IN_BUTTON_A || r.cmd == IN_BUTTON_B) && r.payload.len() >= 2 {
                        let idx = r.payload[0];
                        let val = r.payload[1];
                        if (index::DIAL_1..=index::DIAL_3).contains(&idx) {
                            print!("  dial{} {:?}", idx - index::DIAL_1 + 1, DialEvent::from_byte(val));
                        } else {
                            print!("  index {idx} value {val}");
                        }
                    }
                    println!();
                }
            }
        }
    }
    Ok(())
}

fn bridge_ota(file: &std::path::Path) -> Result<()> {
    let image = std::fs::read(file)?;
    let d = Deck::open()?;
    d200x_core::bridge::ota(&d, &image, &mut |line| println!("{line}"))?;
    Ok(())
}
