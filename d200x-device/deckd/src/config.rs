//! Deck configuration on the device: JSON at /data/deckd/config.json.
//! The model itself lives in the shared `deck-config` crate.

pub use deck_config::*;

pub const DIR: &str = "/data/deckd";
pub const CONFIG_PATH: &str = "/data/deckd/config.json";
pub const ICON_DIR: &str = "/data/deckd/icons";
/// Transient images (names starting with "ram-"): tmpfs, gone at reboot, never written to flash.
pub const RAM_ICON_DIR: &str = "/tmp/deckd-icons";

pub fn load() -> (Config, String) {
    match std::fs::read(CONFIG_PATH) {
        Ok(bytes) => match serde_json::from_slice::<Config>(&bytes) {
            Ok(c) => (c, format!("loaded {} ({} bytes)", CONFIG_PATH, bytes.len())),
            Err(e) => (Config::default_config(), format!("config parse error, using default: {e}")),
        },
        Err(_) => (Config::default_config(), "no config, using default".into()),
    }
}

pub fn save(cfg: &Config) -> Result<(), String> {
    std::fs::create_dir_all(DIR).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = format!("{CONFIG_PATH}.tmp");
    std::fs::write(&tmp, &json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, CONFIG_PATH).map_err(|e| e.to_string())
}
