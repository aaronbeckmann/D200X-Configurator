//! deckd: standalone deck daemon for the Ulanzi D200X rev 02.
//!
//! Usage: deckd [--seconds N] [--no-usb] [--uart] [--uart-echo]
//!   --seconds N   run N seconds then reboot the deck (test mode); default: run forever
//!   --no-usb      do not switch the gadget to HID (keeps ADB; keyboard output disabled)
//!   --uart        open the console UART agent link even if the config has it off
//!   --uart-echo   console UART link test: answer every frame with itself, no agent handling
//!
//! Agents (PC over USB, ESP32 bridge on the console UART, esp32-bridge/docs/INTERFACE.md):
//!   agent -> deck 0x0210 heartbeat, 0x0211 states, 0x0212 result, 0x0213 log, 0x0214 baud request
//!   deck -> agent 0x0310 host action, 0x0311 hello, 0x0312 beacon (UART), 0x0314 baud reply
//!   PC -> ESP32 through the deck: 0x0220 config / 0x0222 status / 0x0224 command (USB) -> 0x03xx
//!   (UART), replies 0x0221 / 0x0223 / 0x0225 (UART) -> 0x03xx (USB); 0x0220..=0x022f reserved
//!
//! Host config channel on /dev/hidg0 (stock framing, see research/PROTOCOL.md):
//!   0x0003 -> 0x0303 device info JSON (stock compatible)
//!   0x000a brightness ASCII 0..100 (stock compatible)
//!   0x0200 set config: payload = JSON, reply 0x0300 {"ok":true|false,"error":..}
//!   0x0201 set icon:   payload = name NUL png-bytes, reply 0x0300
//!   0x0202 get config: reply 0x0302 JSON
//!   0x0203 reboot

mod canvas;
mod compose;
mod config;
mod display;
mod fb;
mod frame;
mod hid;
mod mcu;
mod render;
mod ring;
mod serial;
mod uart;

use compose::{Frame, Stale};
use config::{Action, Config, LinkKind, Step};
use std::collections::HashMap;
use uart::SerialLink;
use display::Display;
use fb::Fb;
use hid::{KeyQueue, Keyboard, Vendor};
use mcu::{Event, Mcu};
use render::{build_page, EffectLook, Renderer, ViewState};
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Size of the log kept in memory, the part `d200x-tool log` shows.
const LOG_TAIL: usize = 60 * 1024;

/// Log for rare events on flash, and the tail of everything in memory. Traffic only goes to the
/// memory tail (`trace`): a line in the JFFS2 log costs up to 8 ms, wears the flash and the file
/// would grow without limit on the partition that holds the config.
#[derive(Clone)]
pub struct Log(Arc<Mutex<(File, VecDeque<u8>)>>);
impl Log {
    fn new() -> Log {
        let _ = std::fs::create_dir_all(config::DIR);
        let f = File::create("/data/deckd/deckd.log").or_else(|_| File::create("/tmp/deckd.log")).expect("log");
        Log(Arc::new(Mutex::new((f, VecDeque::with_capacity(LOG_TAIL)))))
    }
    fn line(&self, s: &str) {
        self.trace(s);
        let mut log = self.0.lock().unwrap();
        let _ = writeln!(log.0, "{s}");
        let _ = log.0.flush();
    }
    pub fn trace(&self, s: &str) {
        let mut log = self.0.lock().unwrap();
        let tail = &mut log.1;
        tail.extend(s.as_bytes());
        tail.push_back(b'\n');
        let over = tail.len().saturating_sub(LOG_TAIL);
        tail.drain(..over);
    }
    fn tail(&self) -> Vec<u8> {
        self.0.lock().unwrap().1.iter().copied().collect()
    }
}

/// Animations step at the panel's 60 Hz.
const FRAME_MS: u64 = 16;

struct App {
    cfg: Config,
    page: String,
    pressed: u16,
    knob_pressed: u8,
    kbd: KeyQueue,
    vendor: Vendor,
    display: Display,
    status: String,
    dirty: bool,
    log: Log,
    usb_ready: bool,
    calibrate: bool,
    /// Agent-fed states.
    states: std::collections::HashMap<String, String>,
    /// The agent on each link, by `link_index`.
    agents: [Option<AgentInfo>; 2],
    uart: SerialLink,
    uart_baud: u32,
    /// `--uart`: the link is open whatever the config says.
    uart_forced: bool,
    /// Random per start, sent in the beacon so the bridge notices a deckd restart.
    boot_id: String,
    /// Route of each plugin the deck wants states from; a change re-sends hello to the agents.
    route_sig: Vec<(String, Option<LinkKind>)>,
    /// Host action requests in flight: (id, key bit, link, sent at).
    pending: Vec<(u64, u16, LinkKind, Instant)>,
    next_id: u64,
    /// Red flash per key, with expiry.
    flashes: Vec<(u16, Instant)>,
    /// Press animations: key bit -> start time.
    anims: Vec<(u16, Instant)>,
    /// Full-screen image currently shown (Screen action).
    screen: Option<Screen>,
    /// Full-screen effects in flight (overlaid on the page): effect, start time, origin (key centre),
    /// slot of the key that fired it (drawn blank in front of the effect).
    effects: Vec<(deck_config::Effect, Instant, (i32, i32), Option<usize>)>,
    /// Key whose action is running (so an `Action::Effect` starts at that key).
    effect_origin: Option<usize>,
    /// Macro steps still to run, each with the key that started it.
    macro_steps: VecDeque<(Step, Option<usize>)>,
    /// A macro delay is running until then.
    macro_wait: Option<Instant>,
    in_macro: bool,
    /// Page to return to when the auto-switched app loses focus.
    auto_return: Option<String>,
    /// Shortest frame delay among the animated tiles on the current page.
    tile_delay: Option<u64>,
    last_tile_frame: u64,
    t0: Instant,
}

/// An agent heard on a link (heartbeat 0x0210).
struct AgentInfo {
    name: String,
    plugins: Vec<String>,
    /// Plugins the agent reports as not connected to their service (`"ready":{"x":false}`).
    not_ready: Vec<String>,
    /// State prefix -> plugin, in addition to `deck_config::plugin_of_state`.
    prefixes: HashMap<String, String>,
    last_heartbeat: Instant,
}

impl AgentInfo {
    fn serves(&self, plugin: &str) -> bool {
        self.plugins.iter().any(|p| p == plugin) && !self.not_ready.iter().any(|p| p == plugin)
    }
}

const LINKS: [LinkKind; 2] = [LinkKind::Usb, LinkKind::Uart];

/// Configurator <-> ESP32 bridge through the deck: even = request (USB 0x02xx -> UART 0x03xx),
/// odd = reply (UART 0x02xx -> USB 0x03xx). 0x0220 config, 0x0222 status, 0x0224 command (OTA ...),
/// the rest reserved so new bridge commands need no deckd change.
const BRIDGE_CMDS: std::ops::RangeInclusive<u16> = 0x0220..=0x022f;

fn link_index(l: LinkKind) -> usize {
    match l {
        LinkKind::Usb => 0,
        LinkKind::Uart => 1,
    }
}

fn link_label(l: LinkKind) -> &'static str {
    match l {
        LinkKind::Usb => "pc",
        LinkKind::Uart => "esp",
    }
}

/// `"req"` of a forwarded request, echoed in a local error reply.
fn req_of(payload: &[u8]) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(payload).ok().and_then(|v| v.get("req").cloned()).unwrap_or(serde_json::Value::Null)
}

/// A Screen action in progress: a still image, or an animation (`image-fNN.png`).
struct Screen {
    image: String,
    until: Option<Instant>,
    frames: u32,
    delay_ms: u64,
    started: Instant,
    last_frame: u64,
}

impl Screen {
    fn frame(&self, now: Instant) -> u64 {
        now.duration_since(self.started).as_millis() as u64 / self.delay_ms.max(16)
    }
    /// Frame file for now, or the still image.
    fn name(&self, now: Instant) -> String {
        if self.frames > 0 {
            format!("{}-f{:02}.png", self.image, self.frame(now) % self.frames as u64)
        } else {
            self.image.clone()
        }
    }
    fn finished(&self, now: Instant) -> bool {
        match self.until {
            Some(t) => now >= t,
            // animation without a timeout: play once
            None => self.frames > 0 && self.frame(now) >= self.frames as u64,
        }
    }
}

impl App {
    /// Start an effect from key `slot` (None: from the screen centre, e.g. a side button).
    fn start_effect(&mut self, e: &deck_config::Effect, slot: Option<usize>) {
        let origin = slot.map(|s| self.slot_centre(s)).unwrap_or((480, 270));
        self.effects.retain(|(x, _, _, _)| x != e);
        self.effects.push((e.clone(), Instant::now(), origin, slot));
        self.dirty = true;
    }

    fn slot_centre(&self, index: usize) -> (i32, i32) {
        let (x, y, w, h) = self.cfg.layout.slot_rect(index);
        (x + w / 2, y + h / 2)
    }

    fn agent_alive(&self, l: LinkKind) -> bool {
        self.agents[link_index(l)].as_ref().map(|a| a.last_heartbeat.elapsed() < Duration::from_secs(6)).unwrap_or(false)
    }

    fn any_agent_alive(&self) -> bool {
        LINKS.iter().any(|&l| self.agent_alive(l))
    }

    /// Status line: "pc", "esp", "pc+esp" or "no agent".
    fn agent_label(&self) -> String {
        let alive: Vec<&str> = LINKS.iter().filter(|&&l| self.agent_alive(l)).map(|&l| link_label(l)).collect();
        if alive.is_empty() {
            "no agent".into()
        } else {
            alive.join("+")
        }
    }

    fn link_open(&self, l: LinkKind) -> bool {
        match l {
            LinkKind::Usb => self.vendor.is_open(),
            LinkKind::Uart => self.uart.is_open(),
        }
    }

    fn link_send(&mut self, l: LinkKind, cmd: u16, payload: &[u8]) {
        match l {
            LinkKind::Usb => self.vendor.send(cmd, payload),
            LinkKind::Uart => self.uart.send(cmd, payload),
        }
    }

    /// Link whose agent runs this plugin's actions: the first in the plugin's route that is open,
    /// alive, offers the plugin and reports it ready.
    fn route(&self, plugin: &str) -> Option<LinkKind> {
        self.cfg.agents.route(plugin).into_iter().find(|&l| self.link_open(l) && self.agent_alive(l) && self.agents[link_index(l)].as_ref().map(|a| a.serves(plugin)).unwrap_or(false))
    }

    fn plugin_of(&self, state: &str) -> String {
        let prefix = state.split(':').next().unwrap_or("");
        for a in self.agents.iter().flatten() {
            if let Some(p) = a.prefixes.get(prefix) {
                return p.clone();
            }
        }
        deck_config::plugin_of_state(state).to_string()
    }

    /// States are taken only from the agent their plugin routes to, so two agents running the
    /// same plugin never fight over a tile. Nobody routed: any agent may feed it.
    fn accepts_state(&self, state: &str, l: LinkKind) -> bool {
        match self.route(&self.plugin_of(state)) {
            Some(owner) => owner == l,
            None => true,
        }
    }

    /// Recompute the routes of the plugins behind the wanted states. On a change every agent gets
    /// hello again, which makes the new owner send its states. Returns whether hello went out.
    fn check_routes(&mut self) -> bool {
        let mut plugins: Vec<String> = self.cfg.wanted_states().iter().map(|s| self.plugin_of(s)).collect();
        plugins.sort();
        plugins.dedup();
        let sig: Vec<(String, Option<LinkKind>)> = plugins.into_iter().map(|p| {
            let r = self.route(&p);
            (p, r)
        }).collect();
        if sig == self.route_sig {
            return false;
        }
        self.route_sig = sig;
        self.log.trace(&format!("routes: {:?}", self.route_sig));
        self.hello_all();
        true
    }

    fn hello_all(&mut self) {
        for l in LINKS {
            if self.agent_alive(l) {
                self.send_hello(l);
            }
        }
    }

    /// Open, reopen or close the console UART link to match the config (or `--uart`).
    fn apply_uart(&mut self) {
        let want = self.uart_forced || self.cfg.agents.uart.enabled;
        let baud = self.cfg.agents.uart.baud;
        if want && (!self.uart.is_open() || baud != self.uart_baud) {
            self.uart = SerialLink::open(baud, &self.log);
            self.uart_baud = baud;
        } else if !want && self.uart.is_open() {
            self.uart = SerialLink::closed();
            self.agents[link_index(LinkKind::Uart)] = None;
            hid::setprop("ctl.start", "console");
            self.log.line("uart: closed, console shell restarted");
            self.dirty = true;
        }
    }

    fn flash(&mut self, bit: u16) {
        self.flashes.push((bit, Instant::now() + Duration::from_millis(450)));
        self.dirty = true;
    }

    fn send_hello(&mut self, link: LinkKind) {
        let mut icons: Vec<String> = std::fs::read_dir(config::ICON_DIR)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        if let Ok(rd) = std::fs::read_dir(config::RAM_ICON_DIR) {
            icons.extend(rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()));
        }
        let hello = serde_json::json!({
            "firmware": "deckd",
            "version": env!("CARGO_PKG_VERSION"),
            "serial": serial(),
            "states": self.cfg.wanted_states(),
            "icons": icons,
            "link": link_label(link),
        });
        self.link_send(link, 0x0311, hello.to_string().as_bytes());
    }

    /// Beacon on the UART: the bridge may only talk while it hears these (the console is a root
    /// shell whenever deckd is not running).
    fn send_beacon(&mut self) {
        let beacon = serde_json::json!({
            "firmware": "deckd",
            "version": env!("CARGO_PKG_VERSION"),
            "boot": self.boot_id,
            "uptime_ms": self.t0.elapsed().as_millis() as u64,
            "baud": [self.uart_baud],
        });
        self.uart.send(0x0312, beacon.to_string().as_bytes());
    }

    /// Foreground app changed: bring up the matching page, or return from an auto-switched one.
    fn auto_switch(&mut self, app: &str) {
        let target = self
            .cfg
            .order()
            .into_iter()
            .find(|name| self.cfg.pages.get(name).and_then(|p| p.app.as_deref()).map(|pat| deck_config::app_matches(pat, app)).unwrap_or(false));
        match target {
            Some(page) if page != self.page => {
                if self.auto_return.is_none() {
                    self.auto_return = Some(self.page.clone());
                }
                self.log.trace(&format!("app {app}: auto page {page}"));
                self.page = page;
                self.dirty = true;
            }
            Some(_) => {}
            None => {
                if let Some(back) = self.auto_return.take() {
                    if self.cfg.pages.contains_key(&back) {
                        self.log.trace(&format!("app {app}: back to {back}"));
                        self.page = back;
                        self.dirty = true;
                    }
                }
            }
        }
    }

    /// Shortest frame delay among animated tiles on the current page (None: nothing animates).
    fn scan_tile_anims(&mut self) {
        let page = self.cfg.pages.get(&self.page);
        self.tile_delay = page
            .map(|p| p.keys.values().filter_map(|k| k.tile.as_deref()).filter_map(|t| self.states.get(t)).filter_map(|v| deck_config::parse_tile(v)).filter(|(_, f, _)| *f > 0).map(|(_, _, d)| d).min())
            .unwrap_or(None);
    }

    /// Hand the current look of the deck to the render thread.
    fn redraw(&mut self) {
        self.dirty = false;
        if self.calibrate {
            self.display.show(Frame::Calibrate(self.cfg.layout.clone()));
            return;
        }
        let now = Instant::now();
        self.flashes.retain(|(_, until)| *until > now);
        let error_flash = self.flashes.iter().fold(0u16, |a, (b, _)| a | b);
        if let Some(s) = &self.screen {
            self.display.show(Frame::Screen { name: s.name(now), animated: s.frames > 0 });
            return;
        }
        const ANIM_MS: f32 = 220.0;
        self.anims.retain(|(_, t)| (now.duration_since(*t).as_millis() as f32) < ANIM_MS);
        let anim: Vec<(u16, f32)> = self.anims.iter().map(|(b, t)| (*b, (now.duration_since(*t).as_millis() as f32 / ANIM_MS).min(1.0))).collect();
        self.scan_tile_anims();
        let now_ms = self.t0.elapsed().as_millis() as u64;
        self.effects.retain(|(e, t, _, _)| now.duration_since(*t).as_millis() as u64 <= e.duration_ms());
        let effects = self.effects.iter().map(|(e, t, origin, slot)| EffectLook { effect: e.clone(), ms: now.duration_since(*t).as_millis() as u64, origin: *origin, slot: *slot }).collect();
        let agent_label = self.agent_label();
        let v = ViewState {
            page_name: &self.page,
            pressed: self.pressed,
            knob_pressed: self.knob_pressed,
            status: &self.status,
            states: &self.states,
            error_flash,
            now_ms,
            agent_alive: self.any_agent_alive(),
            agent_label: &agent_label,
            anim: &anim,
        };
        let empty = config::Page::default();
        let page = self.cfg.pages.get(&self.page).unwrap_or(&empty);
        self.display.show(Frame::Page(build_page(&self.cfg, page, &v, effects)));
    }

    /// Execute an action. `origin` = key bit for flashes, `event` = "press"/"turn", `delta` for knobs.
    fn run_action_from(&mut self, action: &Action, origin: u16, event: &str, delta: i32) {
        if let Action::Host { plugin, op, args, fallback } = action {
            if let Some(link) = self.route(plugin) {
                let id = self.next_id;
                self.next_id += 1;
                let req = serde_json::json!({
                    "id": id, "plugin": plugin, "op": op, "args": args,
                    "event": event, "delta": delta, "page": self.page,
                });
                self.link_send(link, 0x0310, req.to_string().as_bytes());
                self.pending.push((id, origin, link, Instant::now()));
            } else if let Some(fb) = fallback {
                let fb = (**fb).clone();
                self.run_action_from(&fb, origin, event, delta);
            } else {
                self.flash(origin);
            }
            return;
        }
        self.run_action(action);
    }

    fn run_action(&mut self, action: &Action) {
        match action {
            Action::None => {}
            Action::Screen { image, seconds, frames, delay_ms } => {
                let now = Instant::now();
                let until = if *seconds > 0 { Some(now + Duration::from_secs(*seconds as u64)) } else { None };
                self.screen = Some(Screen { image: image.clone(), until, frames: *frames, delay_ms: *delay_ms, started: now, last_frame: 0 });
                self.dirty = true;
            }
            Action::Effect { effect } => {
                let slot = self.effect_origin;
                self.start_effect(effect, slot)
            }
            Action::Host { fallback, .. } => {
                // reached only via Macro steps: no key origin, run fallback if any
                if let Some(fb) = fallback {
                    let fb = (**fb).clone();
                    self.run_action(&fb);
                }
            }
            Action::Key { keys } => {
                let mut mods = 0u8;
                let mut usage = Vec::new();
                for k in keys {
                    if let Some(m) = hid::modifier_bit(k) {
                        mods |= m;
                    } else if let Some(u) = hid::key_usage(k) {
                        usage.push(u);
                    } else {
                        self.log.line(&format!("unknown key name {k}"));
                    }
                }
                self.kbd.chord(mods, usage);
            }
            Action::Text { text } => self.kbd.text(text),
            Action::Page { page } => {
                if self.cfg.pages.contains_key(page) {
                    self.page = page.clone();
                    self.dirty = true;
                }
            }
            Action::PageNext | Action::PagePrev => {
                let order = self.cfg.order();
                if let Some(pos) = order.iter().position(|p| p == &self.page) {
                    let n = order.len();
                    let next = if matches!(action, Action::PageNext) { (pos + 1) % n } else { (pos + n - 1) % n };
                    self.page = order[next].clone();
                    self.dirty = true;
                }
            }
            Action::Macro { steps } => {
                let origin = self.effect_origin;
                if self.in_macro {
                    // a macro inside a macro: runs in place of its step
                    for s in steps.iter().rev() {
                        self.macro_steps.push_front((s.clone(), origin));
                    }
                } else {
                    self.macro_steps.extend(steps.iter().map(|s| (s.clone(), origin)));
                    self.run_macro_steps();
                }
            }
        }
    }

    /// Run macro steps until the next delay. Steps wait for the keyboard, so that what they do
    /// happens in order; meanwhile the deck stays live, a macro can sleep for 10 s.
    fn run_macro_steps(&mut self) {
        self.in_macro = true;
        while !self.macro_steps.is_empty() {
            if self.kbd.busy() || self.macro_wait.map(|t| Instant::now() < t).unwrap_or(false) {
                break;
            }
            self.macro_wait = None;
            let Some((step, origin)) = self.macro_steps.pop_front() else { break };
            match step {
                Step::Delay { delay_ms } => self.macro_wait = Some(Instant::now() + Duration::from_millis(delay_ms.min(10000))),
                Step::Act(a) => {
                    self.effect_origin = origin;
                    self.run_action_from(&a, 0, "press", 0);
                    self.effect_origin = None;
                }
            }
        }
        self.in_macro = false;
    }

    fn handle_event(&mut self, ev: Event) {
        let page = self.cfg.pages.get(&self.page);
        match ev {
            Event::Key { index, pressed } => {
                let bit = if index == 13 { 1 } else { 0x4000 >> index };
                if pressed {
                    self.pressed |= bit;
                } else {
                    self.pressed &= !bit;
                    self.anims.retain(|(b, _)| *b != bit);
                    self.anims.push((bit, Instant::now()));
                }
                let key = page.and_then(|p| p.keys.get(&index.to_string())).map(|k| (k.effect.clone(), k.action.clone()));
                self.dirty = true;
                if pressed && self.screen.is_some() {
                    self.screen = None;
                    return;
                }
                if pressed {
                    if let Some((effect, a)) = key {
                        if let Some(e) = &effect {
                            self.start_effect(e, Some(index as usize));
                        }
                        self.effect_origin = Some(index as usize);
                        self.run_action_from(&a, bit, "press", 0);
                        self.effect_origin = None;
                    }
                }
            }
            Event::Side { index, pressed } => {
                if pressed {
                    let a = if index == 0 { self.cfg.side_left.clone() } else { self.cfg.side_right.clone() };
                    self.run_action_from(&a, 0, "press", 0);
                }
            }
            Event::KnobPress { knob, pressed } => {
                if pressed {
                    self.knob_pressed |= 1 << knob;
                } else {
                    self.knob_pressed &= !(1 << knob);
                }
                let action = page.and_then(|p| p.knobs.get(knob as usize)).map(|k| k.press.clone());
                self.dirty = true;
                if pressed {
                    if let Some(a) = action {
                        self.run_action_from(&a, 1, "press", 0);
                    }
                }
            }
            Event::KnobTurn { knob, dir } => {
                if let Some(k) = page.and_then(|p| p.knobs.get(knob as usize)) {
                    let a = if dir > 0 { k.right.clone() } else { k.left.clone() };
                    self.run_action_from(&a, 1, "turn", dir as i32);
                }
            }
            Event::Version(_) | Event::Other { .. } => {}
        }
    }

    fn handle_host(&mut self, link: LinkKind, msg: hid::Message) {
        // The bridge may run plugins, not reconfigure or reboot the deck.
        let usb_only = matches!(msg.cmd, 0x0003 | 0x000a | 0x0200..=0x0205) || (BRIDGE_CMDS.contains(&msg.cmd) && msg.cmd % 2 == 0);
        if link == LinkKind::Uart && usb_only {
            self.log.trace(&format!("uart: cmd 0x{:04x} is USB only, ignored", msg.cmd));
            return;
        }
        match msg.cmd {
            0x0003 => {
                let info = format!(
                    "{{\"SerialNumber\":\"{}\",\"Dversion\":\"deckd-0.1.0\",\"error\":\"0\",\"DeviceType\":\"D200X\",\"HardwareVersion\":\"SSD210V100\",\"firmware\":\"deckd\"}}",
                    serial()
                );
                let mut payload = info.into_bytes();
                payload.resize(1014, 0);
                self.vendor.send(0x0303, &payload);
            }
            0x000a => {
                if let Ok(s) = std::str::from_utf8(&msg.payload) {
                    if let Ok(v) = s.trim_end_matches('\0').trim().parse::<u8>() {
                        set_brightness(v);
                        self.cfg.brightness = v;
                    }
                }
            }
            0x0200 => {
                let reply = match serde_json::from_slice::<Config>(&msg.payload) {
                    Ok(c) => match config::save(&c) {
                        Ok(()) => {
                            self.cfg = c;
                            if !self.cfg.pages.contains_key(&self.page) {
                                self.page = self.cfg.home.clone();
                            }
                            self.display.stale(Stale::AllIcons);
                            self.dirty = true;
                            self.status = "config updated".into();
                            self.apply_uart();
                            // wanted states changed: every agent needs the new list
                            if !self.check_routes() {
                                self.hello_all();
                            }
                            "{\"ok\":true}".to_string()
                        }
                        Err(e) => format!("{{\"ok\":false,\"error\":\"save: {e}\"}}"),
                    },
                    Err(e) => format!("{{\"ok\":false,\"error\":\"{}\"}}", e.to_string().replace('"', "'")),
                };
                self.log.line(&format!("set config -> {reply}"));
                self.vendor.send(0x0300, reply.as_bytes());
            }
            0x0201 => {
                let reply = match msg.payload.iter().position(|&b| b == 0) {
                    Some(p) => {
                        let name = String::from_utf8_lossy(&msg.payload[..p]).to_string();
                        let safe = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
                        if !safe || name.is_empty() {
                            "{\"ok\":false,\"error\":\"bad icon name\"}".to_string()
                        } else {
                            let ram = name.starts_with("ram-");
                            let dir = if ram { config::RAM_ICON_DIR } else { config::ICON_DIR };
                            let _ = std::fs::create_dir_all(dir);
                            if ram {
                                prune_ram_icons(8 * 1024 * 1024);
                            }
                            match std::fs::write(format!("{dir}/{name}"), &msg.payload[p + 1..]) {
                                Ok(()) => {
                                    self.display.stale(if ram {
                                        Stale::Icon(name)
                                    } else if name.ends_with(".ttf") {
                                        Stale::Fonts
                                    } else {
                                        Stale::AllIcons
                                    });
                                    self.dirty = true;
                                    "{\"ok\":true}".to_string()
                                }
                                Err(e) => format!("{{\"ok\":false,\"error\":\"{e}\"}}"),
                            }
                        }
                    }
                    None => "{\"ok\":false,\"error\":\"no name\"}".to_string(),
                };
                self.log.trace(&format!("set icon -> {reply}"));
                self.vendor.send(0x0300, reply.as_bytes());
            }
            0x0202 => {
                let json = serde_json::to_vec(&self.cfg).unwrap_or_default();
                self.vendor.send(0x0302, &json);
            }
            0x0203 => {
                self.log.line("reboot requested by host");
                let _ = std::process::Command::new("/bin/reboot").status();
            }
            0x0210 => {
                // agent heartbeat: {"agent","plugins":[..],"ready":{plugin:bool},"prefixes":{prefix:plugin}}
                let first = !self.agent_alive(link);
                let v = serde_json::from_slice::<serde_json::Value>(&msg.payload).unwrap_or_default();
                let strings = |key: &str| -> Vec<String> { v.get(key).and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
                let info = AgentInfo {
                    name: v.get("agent").and_then(|a| a.as_str()).unwrap_or("agent").to_string(),
                    plugins: strings("plugins"),
                    not_ready: v.get("ready").and_then(|r| r.as_object()).map(|m| m.iter().filter(|(_, ok)| ok.as_bool() == Some(false)).map(|(p, _)| p.clone()).collect()).unwrap_or_default(),
                    prefixes: v.get("prefixes").and_then(|r| r.as_object()).map(|m| m.iter().filter_map(|(k, p)| p.as_str().map(|p| (k.clone(), p.to_string()))).collect()).unwrap_or_default(),
                    last_heartbeat: Instant::now(),
                };
                if first {
                    self.log.line(&format!("agent connected on {}: {} {:?}", link_label(link), info.name, info.plugins));
                    self.dirty = true;
                }
                self.agents[link_index(link)] = Some(info);
                if !self.check_routes() && first {
                    self.send_hello(link);
                }
            }
            0x0211 => {
                // partial state map
                if let Ok(serde_json::Value::Object(map)) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                    let mut app_changed: Option<String> = None;
                    for (k, v) in map {
                        if !self.accepts_state(&k, link) {
                            continue;
                        }
                        let s = match v {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        };
                        if self.states.get(&k) == Some(&s) {
                            continue;
                        }
                        if k == "system:app" {
                            app_changed = Some(s.clone());
                        }
                        self.states.insert(k, s);
                        self.dirty = true;
                    }
                    if let Some(app) = app_changed {
                        self.auto_switch(&app);
                    }
                }
            }
            0x0212 => {
                // result of a host action
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                    let id = v.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
                    let ok = v.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
                    if let Some(pos) = self.pending.iter().position(|(i, _, _, _)| *i == id) {
                        let (_, bit, _, _) = self.pending.remove(pos);
                        if !ok {
                            self.log.trace(&format!("host action {id} failed: {}", v.get("error").and_then(|e| e.as_str()).unwrap_or("")));
                            self.flash(bit);
                        }
                    }
                }
            }
            0x0205 => {
                // calibration pattern on ("1") / off ("0")
                self.calibrate = msg.payload.first() == Some(&b'1');
                self.dirty = true;
                self.log.line(&format!("calibration {}", self.calibrate));
                self.vendor.send(0x0300, b"{\"ok\":true}");
            }
            0x0204 => {
                // log tail -> 0x0304
                let data = self.log.tail();
                self.vendor.send(0x0304, &data);
            }
            0x0213 => {
                // agent log line {"level","msg"}
                let v = serde_json::from_slice::<serde_json::Value>(&msg.payload).unwrap_or_default();
                let level = v.get("level").and_then(|l| l.as_str()).unwrap_or("info");
                let line = format!("{}: {} {}", link_label(link), level, v.get("msg").and_then(|m| m.as_str()).unwrap_or(""));
                if level == "info" || level == "debug" {
                    self.log.trace(&line);
                } else {
                    self.log.line(&line);
                }
            }
            0x0214 => {
                // rate change request: not supported yet, the answer names the rate that stays
                self.link_send(link, 0x0314, format!("{{\"baud\":{}}}", self.uart_baud).as_bytes());
            }
            c if BRIDGE_CMDS.contains(&c) && c % 2 == 0 => {
                // configurator -> ESP32 bridge (config, status, command), kept in RAM only (secrets)
                if self.agent_alive(LinkKind::Uart) {
                    self.uart.send(msg.cmd + 0x0100, &msg.payload);
                } else {
                    let err = serde_json::json!({ "req": req_of(&msg.payload), "ok": false, "error": "no uart agent" });
                    self.vendor.send(msg.cmd + 0x0101, err.to_string().as_bytes());
                }
            }
            c if BRIDGE_CMDS.contains(&c) => {
                // ESP32 bridge reply -> configurator
                if link == LinkKind::Uart {
                    self.vendor.send(msg.cmd + 0x0100, &msg.payload);
                }
            }
            other => self.log.line(&format!("host cmd 0x{other:04x} len {} ignored", msg.payload.len())),
        }
    }
}

/// Random id for this deckd start (beacon `boot`).
fn boot_id() -> String {
    let mut b = [0u8; 4];
    if let Ok(mut f) = File::open("/dev/urandom") {
        let _ = std::io::Read::read_exact(&mut f, &mut b);
    }
    format!("{:08x}", u32::from_le_bytes(b))
}

fn serial() -> String {
    std::fs::read_to_string("/sys/class/zkswe_usb/zkswe0/iSerial").map(|s| s.trim().to_string()).unwrap_or_default()
}

fn set_brightness(percent: u8) {
    let max: u32 = std::fs::read_to_string("/sys/class/backlight/soc:backlight/max_brightness")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(255);
    let v = (percent.min(100) as u32 * max / 100).max(if percent == 0 { 0 } else { 1 });
    let _ = std::fs::write("/sys/class/backlight/soc:backlight/brightness", v.to_string());
}

/// Keep the tmpfs icon store under `limit` bytes by deleting the oldest files.
fn prune_ram_icons(limit: u64) {
    let Ok(rd) = std::fs::read_dir(config::RAM_ICON_DIR) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, std::path::PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= limit {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= limit * 3 / 4 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut seconds: Option<u64> = None;
    let mut use_usb = true;
    let mut uart_forced = false;
    let mut uart_echo = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--seconds" => {
                seconds = args.get(i + 1).and_then(|s| s.parse().ok());
                i += 1;
            }
            "--no-usb" => use_usb = false,
            "--uart" => uart_forced = true,
            "--uart-echo" => {
                uart_forced = true;
                uart_echo = true;
            }
            _ => {}
        }
        i += 1;
    }
    let log = Log::new();
    log.line(&format!("deckd start args={:?}", &args[1..]));
    hid::setprop("sys.zkapp.state", "running");

    let (cfg, msg) = config::load();
    log.line(&msg);
    let (renderer, fmsg) = Renderer::new();
    log.line(&fmsg);
    let fb = match Fb::open() {
        Ok(f) => Some(f),
        Err(e) => {
            log.line(&format!("fb0: {e}"));
            None
        }
    };
    let mut mcu = match Mcu::open() {
        Ok(m) => m,
        Err(e) => {
            log.line(&format!("ttyS1: {e}"));
            std::process::exit(1);
        }
    };
    set_brightness(cfg.brightness);
    mcu.query_version();
    mcu.query_key_status();

    let display = Display::start(renderer, fb, log.clone());
    let mut app = App {
        page: cfg.home.clone(),
        cfg,
        pressed: 0,
        knob_pressed: 0,
        kbd: KeyQueue::start(Keyboard::closed()),
        vendor: Vendor::closed(),
        display,
        status: "starting".into(),
        dirty: true,
        log,
        usb_ready: false,
        calibrate: false,
        states: std::collections::HashMap::new(),
        agents: [None, None],
        uart: SerialLink::closed(),
        uart_baud: 0,
        uart_forced,
        boot_id: boot_id(),
        route_sig: Vec::new(),
        pending: Vec::new(),
        next_id: 1,
        flashes: Vec::new(),
        anims: Vec::new(),
        screen: None,
        effects: Vec::new(),
        effect_origin: None,
        macro_steps: VecDeque::new(),
        macro_wait: None,
        in_macro: false,
        auto_return: None,
        tile_delay: None,
        last_tile_frame: 0,
        t0: Instant::now(),
    };
    app.redraw();
    app.apply_uart();
    // init may start the console shell again after ctl.stop: look a few more times
    let mut console_checks: VecDeque<Duration> = [2, 5, 10, 30].iter().map(|s| Duration::from_secs(*s)).collect();
    let mut last_beacon = Instant::now() - Duration::from_secs(2);
    let mut uart_stats = (0u64, 0u64);

    if use_usb {
        // The stock app writes the real serial into the gadget's iSerial before
        // switching. It comes from the SPI NOR OTP page 3 (see serial.rs); an
        // override file wins if present.
        let serial = std::fs::read_to_string("/data/deckd/serial")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(serial::serial_number);
        for setup in [false, true] {
            match serial::read_sec_page_raw(3, 17, setup) {
                Ok(b) => app.log.line(&format!("otp page3 (setup={setup}): {}", b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" "))),
                Err(e) => app.log.line(&format!("otp page3 (setup={setup}): error {e}")),
            }
        }
        match &serial {
            Some(s) => match std::fs::write("/sys/class/zkswe_usb/zkswe0/iSerial", s) {
                Ok(()) => app.log.line(&format!("iSerial set to {s}")),
                Err(e) => app.log.line(&format!("iSerial write failed: {e}")),
            },
            None => app.log.line("serial not available"),
        }
        hid::switch_gadget("hid");
        std::thread::sleep(Duration::from_millis(1500));
        let kbd = Keyboard::open();
        app.usb_ready = kbd.is_open();
        app.kbd = KeyQueue::start(kbd);
        app.vendor = Vendor::open();
        app.log.line(&format!("usb hid: keyboard {} vendor {}", app.usb_ready, app.vendor.is_open()));
    }
    app.status = if app.usb_ready { "usb ok".into() } else { "no usb".into() };
    app.dirty = true;

    // zkdaemon watches this property; spawning setprop takes 5 ms, not on the input path
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(10));
        hid::setprop("sys.zkapp.state", "running");
    });

    let start = Instant::now();
    let mut last_redraw = Instant::now();
    let mut last_input = Instant::now();
    // 0 = full brightness, 1 = dimmed, 2 = off
    let mut power_state = 0u8;
    loop {
        if let Some(s) = seconds {
            if start.elapsed() >= Duration::from_secs(s) {
                break;
            }
        }
        // wait for input on MCU or vendor channel, or until the next animation step is due
        let mut fds = vec![libc::pollfd { fd: mcu.fd(), events: libc::POLLIN, revents: 0 }];
        if let Some(vfd) = app.vendor.fd() {
            fds.push(libc::pollfd { fd: vfd, events: libc::POLLIN, revents: 0 });
        }
        if let Some(ufd) = app.uart.fd() {
            fds.push(libc::pollfd { fd: ufd, events: libc::POLLIN, revents: 0 });
        }
        let animating = !app.flashes.is_empty() || !app.anims.is_empty() || !app.effects.is_empty() || app.screen.as_ref().map(|s| s.frames > 0).unwrap_or(false);
        let tile_step = app.tile_delay.map(|d| (d / 14).max(FRAME_MS));
        let mut wait = if animating {
            FRAME_MS - last_redraw.elapsed().as_millis() as u64 % FRAME_MS
        } else if let Some(step) = tile_step {
            step - app.t0.elapsed().as_millis() as u64 % step
        } else {
            100
        };
        if !app.macro_steps.is_empty() {
            wait = wait.min(2);
        }
        unsafe {
            libc::poll(fds.as_mut_ptr(), fds.len() as _, wait as _);
        }
        let events = mcu.poll();
        if !events.is_empty() {
            last_input = Instant::now();
            if power_state != 0 {
                // wake: restore brightness; a press that only wakes the deck is still executed
                set_brightness(app.cfg.brightness);
                power_state = 0;
                app.dirty = true;
            }
        }
        for ev in events {
            app.log.trace(&format!("{} ms {:?}", start.elapsed().as_millis(), ev));
            match ev {
                Event::Version(v) => app.status = format!("mcu {v}"),
                Event::Other { .. } => {}
                ev => app.handle_event(ev),
            }
        }
        let idle = last_input.elapsed().as_secs() as u32;
        if power_state == 0 && app.cfg.dim_after_s > 0 && idle >= app.cfg.dim_after_s {
            set_brightness(app.cfg.dim_brightness);
            power_state = 1;
        }
        if power_state < 2 && app.cfg.off_after_s > 0 && idle >= app.cfg.off_after_s {
            set_brightness(0);
            power_state = 2;
        }
        for m in app.vendor.poll() {
            app.log.trace(&format!("{} ms host cmd 0x{:04x} len {}", start.elapsed().as_millis(), m.cmd, m.payload.len()));
            app.handle_host(LinkKind::Usb, m);
        }
        if app.uart.is_open() {
            for m in app.uart.poll() {
                if uart_echo {
                    app.uart.send(m.cmd, &m.payload);
                } else {
                    app.log.trace(&format!("{} ms uart cmd 0x{:04x} len {}", start.elapsed().as_millis(), m.cmd, m.payload.len()));
                    app.handle_host(LinkKind::Uart, m);
                }
            }
            if !uart_echo && last_beacon.elapsed() >= Duration::from_secs(1) {
                last_beacon = Instant::now();
                app.send_beacon();
            }
            if console_checks.front().map(|d| start.elapsed() >= *d).unwrap_or(false) {
                console_checks.pop_front();
                if uart::stop_console(&app.log) > 0 {
                    app.log.line("uart: console shell had come back");
                }
            }
            let stats = (app.uart.errors(), app.uart.dropped);
            if stats != uart_stats {
                uart_stats = stats;
                app.log.trace(&format!("uart: {} bad frames, {} dropped; {}", stats.0, stats.1, uart::counters()));
            }
        }
        app.run_macro_steps();
        // host actions without a result within 15 s: flash; agent gone: redraw the dot
        let now = Instant::now();
        let timed_out: Vec<u16> = app.pending.iter().filter(|(_, _, _, t)| now.duration_since(*t) > Duration::from_secs(15)).map(|(_, b, _, _)| *b).collect();
        if !timed_out.is_empty() {
            app.pending.retain(|(_, _, _, t)| now.duration_since(*t) <= Duration::from_secs(15));
            for b in timed_out {
                app.flash(b);
            }
        }
        // animations step once per frame, however often input wakes this loop
        if (!app.flashes.is_empty() || !app.anims.is_empty() || !app.effects.is_empty()) && last_redraw.elapsed().as_millis() as u64 >= FRAME_MS {
            app.dirty = true;
        }
        if let Some(step) = tile_step {
            let f = app.t0.elapsed().as_millis() as u64 / step;
            if f != app.last_tile_frame {
                app.last_tile_frame = f;
                app.dirty = true;
            }
        }
        if let Some(s) = app.screen.as_mut() {
            let now = Instant::now();
            if s.finished(now) {
                app.screen = None;
                app.dirty = true;
            } else if s.frames > 0 {
                let f = s.frame(now);
                if f != s.last_frame {
                    s.last_frame = f;
                    app.dirty = true;
                }
            }
        }
        for l in LINKS {
            if app.agents[link_index(l)].is_some() && !app.agent_alive(l) {
                app.agents[link_index(l)] = None;
                app.log.line(&format!("agent lost on {}", link_label(l)));
                app.dirty = true;
                app.check_routes();
            }
        }
        if app.dirty {
            app.redraw();
            last_redraw = Instant::now();
        }
    }
    app.log.line("test run finished, rebooting");
    app.display.show(Frame::Black);
    app.display.wait_idle();
    let _ = std::process::Command::new("/bin/reboot").status();
}
