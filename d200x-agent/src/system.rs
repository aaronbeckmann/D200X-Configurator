//! `system` plugin: things only the PC can do.
//! ops: open {path, args?}, url {url}, close {process}, media {key}, text? (no: deck does typing)

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Foreground application watcher: publishes `system:app` (exe name, lowercase)
/// and `system:title` (window title). Also samples general system info (`INFO_STATES`).
pub struct Foreground {
    app: Arc<Mutex<String>>,
    title: Arc<Mutex<String>>,
    changed: Arc<Mutex<bool>>,
    info: Arc<Mutex<BTreeMap<String, String>>>,
}

/// General system info, refreshed every 2 s. Battery states only exist on a PC with one.
pub const INFO_STATES: [&str; 16] = [
    "system:cpu", "system:ram", "system:ram_used", "system:ram_total", "system:disk", "system:disk_free", "system:uptime", "system:uptime_h",
    "system:time", "system:date", "system:weekday", "system:host", "system:volume", "system:mute", "system:battery", "system:charging",
];

impl Foreground {
    pub fn start() -> Foreground {
        let app = Arc::new(Mutex::new(String::new()));
        let title = Arc::new(Mutex::new(String::new()));
        let changed = Arc::new(Mutex::new(false));
        #[cfg(windows)]
        {
            let (a, t, c) = (app.clone(), title.clone(), changed.clone());
            std::thread::Builder::new()
                .name("foreground".into())
                .spawn(move || loop {
                    let (exe, ttl) = win_foreground();
                    {
                        let mut ca = a.lock().unwrap();
                        let mut ct = t.lock().unwrap();
                        if *ca != exe || *ct != ttl {
                            *ca = exe;
                            *ct = ttl;
                            *c.lock().unwrap() = true;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(400));
                })
                .expect("foreground thread");
        }
        let info = Arc::new(Mutex::new(BTreeMap::new()));
        #[cfg(windows)]
        {
            let (i, c) = (info.clone(), changed.clone());
            std::thread::Builder::new()
                .name("sysinfo".into())
                .spawn(move || {
                    let mut cpu = info::Cpu::default();
                    loop {
                        let now = info::sample(&mut cpu);
                        let mut cur = i.lock().unwrap();
                        if *cur != now {
                            *cur = now;
                            *c.lock().unwrap() = true;
                        }
                        drop(cur);
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                })
                .expect("sysinfo thread");
        }
        Foreground { app, title, changed, info }
    }
    pub fn take_changed(&self) -> bool {
        std::mem::replace(&mut *self.changed.lock().unwrap(), false)
    }
    pub fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "system:app" => Some(self.app.lock().unwrap().clone()),
            "system:title" => Some(self.title.lock().unwrap().clone()),
            _ => self.info.lock().unwrap().get(name).cloned(),
        }
    }
}

#[cfg(windows)]
fn win_foreground() -> (String, String) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return (String::new(), String::new());
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let mut title_buf = [0u16; 256];
        let n = GetWindowTextW(hwnd, title_buf.as_mut_ptr(), title_buf.len() as i32);
        let title = String::from_utf16_lossy(&title_buf[..n.max(0) as usize]);
        let mut exe = String::new();
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if !h.is_null() {
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0 {
                let full = String::from_utf16_lossy(&buf[..len as usize]);
                exe = full.rsplit(|c| c == '\\' || c == '/').next().unwrap_or("").to_lowercase();
            }
            CloseHandle(h);
        }
        (exe, title)
    }
}

pub fn run(op: &str, args: &Value) -> Result<()> {
    match op {
        "open" => {
            let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("open: missing path"))?;
            let extra: Vec<String> = args
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if extra.is_empty() {
                open::that_detached(path).map_err(|e| anyhow!("open {path}: {e}"))?;
            } else {
                std::process::Command::new(path).args(extra).spawn().map_err(|e| anyhow!("spawn {path}: {e}"))?;
            }
            Ok(())
        }
        "url" => {
            let url = args.get("url").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("url: missing url"))?;
            open::that_detached(url).map_err(|e| anyhow!("open {url}: {e}"))
        }
        "close" => {
            let proc_name = args.get("process").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("close: missing process"))?;
            let safe = proc_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
            if !safe {
                return Err(anyhow!("close: bad process name"));
            }
            #[cfg(windows)]
            {
                let st = std::process::Command::new("taskkill").args(["/IM", proc_name, "/F"]).status()?;
                if !st.success() {
                    return Err(anyhow!("taskkill {proc_name} failed"));
                }
            }
            Ok(())
        }
        "media" => {
            let key = args.get("key").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("media: missing key"))?;
            media_key(key)
        }
        other => Err(anyhow!("system: unknown op {other}")),
    }
}

#[cfg(windows)]
pub fn media_key(key: &str) -> Result<()> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP};
    let vk: u8 = match key {
        "play_pause" | "play" | "pause" => 0xB3,
        "next" => 0xB0,
        "prev" | "previous" => 0xB1,
        "stop" => 0xB2,
        "mute" => 0xAD,
        "volume_up" => 0xAF,
        "volume_down" => 0xAE,
        other => return Err(anyhow!("media: unknown key {other}")),
    };
    unsafe {
        keybd_event(vk, 0, 0, 0);
        keybd_event(vk, 0, KEYEVENTF_KEYUP, 0);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn media_key(_key: &str) -> Result<()> {
    Err(anyhow!("media keys not implemented on this OS"))
}

/// System info sampling (Windows APIs, no extra processes).
#[cfg(windows)]
mod info {
    use std::collections::BTreeMap;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows_sys::Win32::System::Threading::GetSystemTimes;

    /// CPU times of the previous sample: usage is the share of busy time in between.
    #[derive(Default)]
    pub struct Cpu {
        idle: u64,
        total: u64,
    }

    fn ft(f: &FILETIME) -> u64 {
        (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64
    }

    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    pub fn sample(cpu: &mut Cpu) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        let mut put = |k: &str, v: String| {
            m.insert(format!("system:{k}"), v);
        };
        unsafe {
            let (mut idle, mut kernel, mut user): (FILETIME, FILETIME, FILETIME) = (std::mem::zeroed(), std::mem::zeroed(), std::mem::zeroed());
            if GetSystemTimes(&mut idle, &mut kernel, &mut user) != 0 {
                // kernel time includes idle time
                let (idle, total) = (ft(&idle), ft(&kernel) + ft(&user));
                if cpu.total > 0 && total > cpu.total {
                    let busy = 1.0 - (idle - cpu.idle) as f64 / (total - cpu.total) as f64;
                    put("cpu", format!("{:.0}", (busy * 100.0).clamp(0.0, 100.0)));
                }
                *cpu = Cpu { idle, total };
            }
            let mut mem: MEMORYSTATUSEX = std::mem::zeroed();
            mem.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut mem) != 0 {
                let used = (mem.ullTotalPhys - mem.ullAvailPhys) as f64;
                put("ram", mem.dwMemoryLoad.to_string());
                put("ram_used", format!("{:.1}", used / GB));
                put("ram_total", format!("{:.0}", mem.ullTotalPhys as f64 / GB));
            }
            let root: Vec<u16> = "C:\\".encode_utf16().chain(std::iter::once(0)).collect();
            let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
            if GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, &mut total_free) != 0 && total > 0 {
                put("disk", format!("{:.0}", 100.0 * (total - total_free) as f64 / total as f64));
                put("disk_free", format!("{:.0}", free as f64 / GB));
            }
            let up = GetTickCount64() / 1000;
            put("uptime_h", format!("{:.1}", up as f64 / 3600.0));
            put("uptime", if up >= 86400 { format!("{}d {}h", up / 86400, up % 86400 / 3600) } else { format!("{}h {:02}m", up / 3600, up % 3600 / 60) });
            let mut t = std::mem::zeroed();
            GetLocalTime(&mut t);
            let t: windows_sys::Win32::Foundation::SYSTEMTIME = t;
            put("time", format!("{:02}:{:02}", t.wHour, t.wMinute));
            put("date", format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay));
            put("weekday", ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"].get(t.wDayOfWeek as usize).unwrap_or(&"").to_string());
            let mut ps: SYSTEM_POWER_STATUS = std::mem::zeroed();
            // BatteryFlag 128: no battery, 255: unknown
            if GetSystemPowerStatus(&mut ps) != 0 && ps.BatteryFlag & 128 == 0 && ps.BatteryFlag != 255 && ps.BatteryLifePercent <= 100 {
                put("battery", ps.BatteryLifePercent.to_string());
                put("charging", (ps.ACLineStatus == 1).to_string());
            }
        }
        put("host", std::env::var("COMPUTERNAME").unwrap_or_default());
        if let Some((vol, mute)) = volume() {
            put("volume", format!("{:.0}", vol * 100.0));
            put("mute", mute.to_string());
        }
        m
    }

    /// Master volume (0..1) and mute of the default playback device.
    fn volume() -> Option<(f32, bool)> {
        use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
        use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
        use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
        unsafe {
            // once per thread; later calls only return S_FALSE
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
            let ep: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None).ok()?;
            Some((ep.GetMasterVolumeLevelScalar().ok()?, ep.GetMute().ok()?.as_bool()))
        }
    }
}
