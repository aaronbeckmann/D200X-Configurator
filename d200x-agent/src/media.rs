//! `media` plugin: the Windows system media session (what the OS media keys
//! and the volume flyout control). Works with Spotify, browsers, players.
//! ops: play_pause, play, pause, next, prev, stop
//! states: media:status (playing|paused|stopped|none), media:title, media:artist,
//!         media:line ("Artist – Title"), media:app
//! icon: `media-art.png` is kept up to date with the current album art.

use anyhow::{anyhow, Result};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default, Clone)]
pub struct NowPlaying {
    pub status: String,
    pub title: String,
    pub artist: String,
    pub app: String,
    pub art_hash: u64,
}

pub const STATES: [&str; 5] = ["media:status", "media:title", "media:artist", "media:line", "media:app"];

pub struct Media {
    now: Arc<Mutex<NowPlaying>>,
    art: Arc<Mutex<Option<(u64, Vec<u8>)>>>, // (hash, png) pending upload
    changed: Arc<Mutex<bool>>,
}

impl Media {
    pub fn start(log: Arc<dyn Fn(&str) + Send + Sync>) -> Media {
        let now = Arc::new(Mutex::new(NowPlaying::default()));
        let art = Arc::new(Mutex::new(None));
        let changed = Arc::new(Mutex::new(false));
        #[cfg(windows)]
        {
            let (now2, art2, ch2) = (now.clone(), art.clone(), changed.clone());
            std::thread::Builder::new()
                .name("media".into())
                .spawn(move || {
                    let mut last_art = 0u64;
                    loop {
                        match win::poll() {
                            Ok((np, art_bytes)) => {
                                let mut cur = now2.lock().unwrap();
                                if cur.status != np.status || cur.title != np.title || cur.artist != np.artist || cur.app != np.app {
                                    *cur = np.clone();
                                    *ch2.lock().unwrap() = true;
                                }
                                drop(cur);
                                if let Some(bytes) = art_bytes {
                                    let h = hash(&bytes);
                                    if h != last_art {
                                        last_art = h;
                                        if let Ok(png) = deck_icons::render_bytes(&bytes, 110, 110) {
                                            *art2.lock().unwrap() = Some((h, png));
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                let mut cur = now2.lock().unwrap();
                                if cur.status != "none" {
                                    *cur = NowPlaying { status: "none".into(), ..Default::default() };
                                    *ch2.lock().unwrap() = true;
                                    log(&format!("media: no session ({e})"));
                                }
                            }
                        }
                        std::thread::sleep(Duration::from_millis(1000));
                    }
                })
                .expect("media thread");
        }
        Media { now, art, changed }
    }

    pub fn take_changed(&self) -> bool {
        std::mem::replace(&mut *self.changed.lock().unwrap(), false)
    }

    pub fn take_art(&self) -> Option<Vec<u8>> {
        self.art.lock().unwrap().take().map(|(_, p)| p)
    }

    pub fn resolve(&self, name: &str) -> Option<String> {
        let np = self.now.lock().unwrap();
        Some(match name {
            "media:status" => np.status.clone(),
            "media:title" => np.title.clone(),
            "media:artist" => np.artist.clone(),
            "media:app" => np.app.clone(),
            "media:line" => {
                if np.artist.is_empty() {
                    np.title.clone()
                } else {
                    format!("{} - {}", np.artist, np.title)
                }
            }
            _ => return None,
        })
    }

    pub fn run(&self, op: &str) -> Result<()> {
        #[cfg(windows)]
        {
            return win::control(op);
        }
        #[allow(unreachable_code)]
        Err(anyhow!("media control not available on this OS"))
    }
}

fn hash(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &x in b.iter().step_by(7) {
        h ^= x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h ^ (b.len() as u64)
}

#[cfg(windows)]
mod win {
    use super::NowPlaying;
    use anyhow::{anyhow, Result};
    use windows::Media::Control::{GlobalSystemMediaTransportControlsSessionManager as Manager, GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status};
    use windows::Storage::Streams::DataReader;

    pub fn poll() -> Result<(NowPlaying, Option<Vec<u8>>)> {
        let mgr = Manager::RequestAsync()?.get()?;
        let session = mgr.GetCurrentSession().map_err(|_| anyhow!("no media session"))?;
        let props = session.TryGetMediaPropertiesAsync()?.get()?;
        let info = session.GetPlaybackInfo()?;
        let status = match info.PlaybackStatus()? {
            Status::Playing => "playing",
            Status::Paused => "paused",
            Status::Stopped => "stopped",
            _ => "none",
        };
        let np = NowPlaying {
            status: status.into(),
            title: props.Title().map(|s| s.to_string()).unwrap_or_default(),
            artist: props.Artist().map(|s| s.to_string()).unwrap_or_default(),
            app: session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default(),
            art_hash: 0,
        };
        let art = (|| -> Result<Vec<u8>> {
            let thumb = props.Thumbnail()?;
            let stream = thumb.OpenReadAsync()?.get()?;
            let size = stream.Size()? as u32;
            if size == 0 || size > 4_000_000 {
                return Err(anyhow!("no art"));
            }
            let reader = DataReader::CreateDataReader(&stream)?;
            reader.LoadAsync(size)?.get()?;
            let mut buf = vec![0u8; size as usize];
            reader.ReadBytes(&mut buf)?;
            Ok(buf)
        })()
        .ok();
        Ok((np, art))
    }

    pub fn control(op: &str) -> Result<()> {
        let mgr = Manager::RequestAsync()?.get()?;
        let session = mgr.GetCurrentSession().map_err(|_| anyhow!("no media session"))?;
        let ok = match op {
            "play_pause" => session.TryTogglePlayPauseAsync()?.get()?,
            "play" => session.TryPlayAsync()?.get()?,
            "pause" => session.TryPauseAsync()?.get()?,
            "next" => session.TrySkipNextAsync()?.get()?,
            "prev" | "previous" => session.TrySkipPreviousAsync()?.get()?,
            "stop" => session.TryStopAsync()?.get()?,
            other => return Err(anyhow!("media: unknown op {other}")),
        };
        if ok {
            Ok(())
        } else {
            Err(anyhow!("media: {op} not accepted by the player"))
        }
    }
}
