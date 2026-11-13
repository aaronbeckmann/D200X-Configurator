//! Icon rendering for the deck. Tile content box is 124x110 at the default
//! layout; icons are rendered to a square that fits (default 96 px) with
//! transparent background and the requested colour.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

pub const DEFAULT_SIZE: u32 = 96;

/// Directory holding the `@mdi/svg` package (`svg/*.svg`, `meta.json`).
pub fn mdi_dir() -> Option<PathBuf> {
    let candidates = [
        std::env::var("D200X_MDI_DIR").ok().map(PathBuf::from),
        std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("assets/mdi"))),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/mdi")),
    ];
    candidates.into_iter().flatten().find(|p| p.join("svg").is_dir())
}

/// All MDI icon names (from the svg directory).
pub fn mdi_names() -> Vec<String> {
    let Some(dir) = mdi_dir() else { return Vec::new() };
    let mut v: Vec<String> = std::fs::read_dir(dir.join("svg"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().map(|s| s.trim_end_matches(".svg").to_string()))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Render an MDI icon (`lightbulb-on`, with or without `mdi:` prefix) in `color` (#rrggbb) to PNG.
pub fn render_mdi(name: &str, color: &str, size: u32) -> Result<Vec<u8>> {
    let name = name.trim_start_matches("mdi:");
    let dir = mdi_dir().ok_or_else(|| anyhow!("MDI icon set not found (assets/mdi)"))?;
    let svg = std::fs::read_to_string(dir.join("svg").join(format!("{name}.svg"))).with_context(|| format!("no MDI icon '{name}'"))?;
    // MDI svgs are a single path without fill; inject the colour on the root element.
    let svg = svg.replacen("<svg ", &format!("<svg fill=\"{color}\" "), 1);
    render_svg(svg.as_bytes(), size)
}

/// Render any SVG to a size x size PNG (aspect preserved, centred).
pub fn render_svg(svg: &[u8], size: u32) -> Result<Vec<u8>> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg, &opt).map_err(|e| anyhow!("svg parse: {e}"))?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or_else(|| anyhow!("pixmap"))?;
    let vs = tree.size();
    let scale = (size as f32 / vs.width()).min(size as f32 / vs.height());
    let tx = (size as f32 - vs.width() * scale) / 2.0;
    let ty = (size as f32 - vs.height() * scale) / 2.0;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| anyhow!("png encode: {e}"))
}

/// Load a PNG/JPG/SVG file and produce a PNG fitting w x h (aspect preserved).
pub fn render_file(path: &Path, w: u32, h: u32) -> Result<Vec<u8>> {
    if path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("svg")).unwrap_or(false) {
        let data = std::fs::read(path)?;
        return render_svg(&data, w.min(h));
    }
    let img = image::open(path).with_context(|| format!("read {}", path.display()))?;
    let img = img.resize(w, h, image::imageops::FilterType::Lanczos3).to_rgba8();
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

/// Icon file name convention for rendered MDI icons.
pub fn mdi_file_name(name: &str, color: &str) -> String {
    format!("mdi-{}-{}.png", name.trim_start_matches("mdi:"), color.trim_start_matches('#').to_ascii_lowercase())
}

/// Default MDI icon for a Home Assistant entity without an explicit icon.
pub fn default_ha_icon(entity_id: &str, device_class: Option<&str>, state: &str) -> &'static str {
    let domain = entity_id.split('.').next().unwrap_or("");
    let on = state == "on";
    match (domain, device_class) {
        ("light", _) => if on { "lightbulb-on" } else { "lightbulb-outline" },
        ("switch", Some("outlet")) => if on { "power-plug" } else { "power-plug-off" },
        ("switch", _) => if on { "toggle-switch" } else { "toggle-switch-off-outline" },
        ("fan", _) => "fan",
        ("climate", _) => "thermostat",
        ("cover", Some("garage")) => if on { "garage-open" } else { "garage" },
        ("cover", _) => "window-shutter",
        ("lock", _) => if state == "locked" { "lock" } else { "lock-open-variant" },
        ("media_player", _) => if state == "playing" { "pause" } else { "play" },
        ("scene", _) => "palette",
        ("script", _) => "script-text",
        ("automation", _) => "robot",
        ("input_boolean", _) => if on { "check-circle" } else { "circle-outline" },
        ("binary_sensor", Some("door")) => if on { "door-open" } else { "door-closed" },
        ("binary_sensor", Some("window")) => if on { "window-open" } else { "window-closed" },
        ("binary_sensor", Some("motion")) => if on { "motion-sensor" } else { "motion-sensor-off" },
        ("binary_sensor", _) => if on { "checkbox-marked-circle" } else { "checkbox-blank-circle-outline" },
        ("sensor", Some("temperature")) => "thermometer",
        ("sensor", Some("humidity")) => "water-percent",
        ("sensor", Some("battery")) => "battery",
        ("sensor", Some("power")) => "flash",
        ("sensor", Some("energy")) => "lightning-bolt",
        ("sensor", Some("illuminance")) => "brightness-5",
        ("sensor", _) => "eye",
        ("button", _) => "gesture-tap-button",
        ("vacuum", _) => "robot-vacuum",
        ("camera", _) => "camera",
        _ => "home-assistant",
    }
}

/// Decode image bytes (PNG/JPEG, e.g. album art) and produce a PNG fitting w x h.
pub fn render_bytes(bytes: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let img = image::load_from_memory(bytes).context("decode image")?;
    let img = img.resize(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Minecraft item textures
// ---------------------------------------------------------------------------

/// Directory holding extracted Minecraft item textures (`<name>.png`, 16x16 pixel art).
/// Filled by [`import_minecraft`] from the player's own client jar.
pub fn minecraft_dir() -> Option<PathBuf> {
    let candidates = [
        std::env::var("D200X_MINECRAFT_DIR").ok().map(PathBuf::from),
        std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("assets/minecraft/item"))),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/minecraft/item")),
        user_minecraft_dir(),
    ];
    candidates.into_iter().flatten().find(|p| p.is_dir())
}

/// Where [`import_minecraft`] writes by default: next to the MDI set in the
/// source tree, or, for an installed copy (no source tree on this PC), the
/// user's `%APPDATA%\d200x\minecraft\item`.
pub fn minecraft_default_dir() -> PathBuf {
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets");
    if assets.join("mdi").is_dir() {
        return assets.join("minecraft/item");
    }
    user_minecraft_dir().unwrap_or_else(|| PathBuf::from("assets/minecraft/item"))
}

/// Per-user location for imported textures (writable without admin rights).
fn user_minecraft_dir() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(|a| PathBuf::from(a).join("d200x").join("minecraft").join("item"))
}

/// All imported Minecraft item names, sorted.
pub fn minecraft_names() -> Vec<String> {
    let Some(dir) = minecraft_dir() else { return Vec::new() };
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_str().and_then(|s| s.strip_suffix(".png")).map(str::to_string)).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Newest Minecraft client jar in the launcher's `versions` folder (release versions preferred over snapshots).
pub fn find_minecraft_jar() -> Option<PathBuf> {
    let base = std::env::var("APPDATA").ok().map(|a| PathBuf::from(a).join(".minecraft")).or_else(|| home_dir().map(|h| h.join(".minecraft")))?;
    let rd = std::fs::read_dir(base.join("versions")).ok()?;
    let mut jars: Vec<(bool, std::time::SystemTime, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let jar = e.path().join(format!("{name}.jar"));
            let m = jar.metadata().ok()?;
            let release = name.chars().all(|c| c.is_ascii_digit() || c == '.');
            Some((release, m.modified().ok()?, jar))
        })
        .collect();
    jars.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    jars.into_iter().next().map(|j| j.2)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("USERPROFILE").ok().or_else(|| std::env::var("HOME").ok()).map(PathBuf::from)
}

/// Extract the item textures from a Minecraft client jar into `out`. Animation strips with
/// numbered frames (`clock_00` .. `clock_63`) collapse to their first frame under the base name.
/// Returns the number of icons written.
pub fn import_minecraft(jar: &Path, out: &Path) -> Result<usize> {
    let file = std::fs::File::open(jar).with_context(|| format!("open {}", jar.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("read jar")?;
    std::fs::create_dir_all(out)?;
    let mut n = 0usize;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let name = f.name().to_string();
        let Some(rest) = name.strip_prefix("assets/minecraft/textures/item/") else { continue };
        let Some(stem) = rest.strip_suffix(".png") else { continue };
        if stem.contains('/') {
            continue;
        }
        // numbered frames: keep frame 00 as the item itself, drop the rest
        let stem = match stem.rsplit_once('_') {
            Some((base, num)) if num.len() == 2 && num.chars().all(|c| c.is_ascii_digit()) => {
                if num != "00" {
                    continue;
                }
                base.to_string()
            }
            _ => stem.to_string(),
        };
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut bytes)?;
        std::fs::write(out.join(format!("{stem}.png")), &bytes)?;
        n += 1;
    }
    Ok(n)
}

/// Render a Minecraft item as a crisp pixel-art PNG of `size` x `size` (nearest-neighbour, integer scale).
pub fn render_minecraft(name: &str, size: u32) -> Result<Vec<u8>> {
    let name = name.trim_start_matches("mc:");
    let dir = minecraft_dir().ok_or_else(|| anyhow!("Minecraft icons not imported yet (Minecraft tab in the library)"))?;
    let img = image::open(dir.join(format!("{name}.png"))).with_context(|| format!("no Minecraft item '{name}'"))?.to_rgba8();
    let (w, h) = img.dimensions();
    let scale = (size / w.max(h)).max(1);
    let (dw, dh) = (w * scale, h * scale);
    let mut out = image::RgbaImage::new(size, size);
    let (ox, oy) = ((size.saturating_sub(dw)) / 2, (size.saturating_sub(dh)) / 2);
    for y in 0..dh.min(size) {
        for x in 0..dw.min(size) {
            out.put_pixel(ox + x, oy + y, *img.get_pixel(x / scale, y / scale));
        }
    }
    let mut buf = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;
    Ok(buf)
}

/// Icon file name convention for Minecraft items.
pub fn minecraft_file_name(name: &str) -> String {
    format!("mc-{}.png", name.trim_start_matches("mc:"))
}

// ---------------------------------------------------------------------------
// Animations (GIF -> frame PNGs)
// ---------------------------------------------------------------------------

/// One rendered animation frame.
pub struct Frame {
    pub png: Vec<u8>,
    pub delay_ms: u64,
}

/// Decode a GIF (or a single PNG/JPG as a one-frame animation) into PNG frames fitting w x h
/// (aspect preserved, transparent padding). At most `max_frames` frames; longer GIFs are
/// sampled evenly so the whole animation is kept.
pub fn render_animation(path: &Path, w: u32, h: u32, max_frames: usize) -> Result<Vec<Frame>> {
    let is_gif = path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("gif")).unwrap_or(false);
    if !is_gif {
        return Ok(vec![Frame { png: render_file(path, w, h)?, delay_ms: 100 }]);
    }
    use image::AnimationDecoder;
    let file = std::fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
    let dec = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(file)).context("decode gif")?;
    let frames: Vec<image::Frame> = dec.into_frames().collect_frames().context("gif frames")?;
    if frames.is_empty() {
        return Err(anyhow!("gif has no frames"));
    }
    let step = (frames.len() as f32 / max_frames.max(1) as f32).max(1.0);
    let mut out = Vec::new();
    let mut acc = 0.0f32;
    let mut carry_ms = 0u64;
    for (i, f) in frames.iter().enumerate() {
        let (num, den) = f.delay().numer_denom_ms();
        let ms = if den == 0 { 100 } else { (num / den) as u64 };
        carry_ms += ms;
        if (i as f32) < acc {
            continue; // skipped frame: its time is added to the next kept one
        }
        acc += step;
        let img = image::DynamicImage::ImageRgba8(f.buffer().clone());
        let img = img.resize(w, h, image::imageops::FilterType::Triangle).to_rgba8();
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
        out.push(Frame { png, delay_ms: carry_ms.max(20) });
        carry_ms = 0;
    }
    Ok(out)
}

/// Frame file name: `<base>-f00.png`.
pub fn frame_file_name(base: &str, i: usize) -> String {
    format!("{base}-f{i:02}.png")
}
