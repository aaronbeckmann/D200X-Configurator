//! D200X Configurator: desktop GUI for the deckd firmware.
//! Layout: top bar (device, agent, save) / left sidebar (pages) / deck canvas /
//! right inspector (appearance, action, states & looks) / bottom library (plugins).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod ha;
mod integrations;
mod presets;
mod states;
mod ui_canvas;
mod ui_color;
mod ui_inspector;
mod ui_bridge;
mod ui_library;
mod ui_states;
mod ui_windows;

use app::{agent_exe, App, Sel};
use deck_config::{Config, Page};
use eframe::egui::{self, Color32, RichText};
use std::sync::atomic::Ordering;

fn main() -> eframe::Result {
    // sign-in autostart entry (app::set_autostart): start the agent hidden, no window
    if std::env::args().any(|a| a == "--start-agent") {
        if !app::agent_process_running() {
            if let Some(p) = agent_exe() {
                let _ = spawn_hidden(&p);
            }
        }
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1360.0, 860.0]).with_min_inner_size([1000.0, 680.0]).with_title("D200X Configurator"),
        ..Default::default()
    };
    eframe::run_native(
        "D200X Configurator",
        options,
        Box::new(|cc| {
            let mut style = (*cc.egui_ctx.style()).clone();
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(10.0, 5.0);
            cc.egui_ctx.set_style(style);
            Ok(Box::new(App::new(cc)))
        }),
    )
}

impl eframe::App for App {
    /// Unsaved edits survive a restart: eframe calls this on close and every 30 s.
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_local();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_connect();
        ui_library::poll_spotify_login(self);
        ui_color::finish_closed_picker(ctx);
        // eframe's own storage is not enabled: keep the local copy fresh while editing and on close
        if (self.dirty && self.last_autosave.elapsed() > std::time::Duration::from_secs(3)) || ctx.input(|i| i.viewport().close_requested()) {
            self.save_local();
            self.last_autosave = std::time::Instant::now();
        }
        // Panels take whatever the deck preview does not need at full size.
        let screen = ctx.screen_rect();
        let left_w = 190.0;
        let top_h = 64.0;
        let right_w = (screen.width() - left_w - ui_canvas::FULL_W).max(380.0);
        let bottom_h = (screen.height() - top_h - ui_canvas::FULL_H).max(220.0);

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("D200X");
                ui.separator();
                let conn = if self.connected { Color32::LIGHT_GREEN } else { Color32::LIGHT_RED };
                ui.label(RichText::new(if self.connected { "deck connected" } else { "deck not connected" }).color(conn));
                if ui.button("Connect").clicked() {
                    self.connect_async(ctx.clone());
                }
                if ui.add_enabled(self.connected, egui::Button::new("Load from deck")).clicked() {
                    self.load_from_deck();
                }
                let save_label = if self.dirty { "Save to deck *" } else { "Save to deck" };
                if ui.add_enabled(self.connected, egui::Button::new(RichText::new(save_label).strong())).clicked() {
                    self.save_to_deck();
                }
                ui.separator();
                let agent_running = self.agent_running.load(Ordering::Relaxed);
                ui.label(RichText::new(if agent_running { "agent running" } else { "agent stopped" }).color(if agent_running { Color32::LIGHT_GREEN } else { Color32::LIGHT_RED }));
                if agent_running {
                    if ui.small_button("stop").clicked() {
                        stop_agent(self);
                    }
                } else if ui.small_button("start").clicked() {
                    start_agent(self);
                }
                let mut auto = self.agent_autostart;
                if ui.checkbox(&mut auto, "autostart").on_hover_text("start the agent when you sign in to Windows (no window)").changed() {
                    match app::set_autostart(auto) {
                        Ok(()) => {
                            self.agent_autostart = auto;
                            if auto && !agent_running {
                                start_agent(self);
                            }
                            self.set_status(if auto { "agent starts with Windows" } else { "agent no longer starts with Windows" }, true);
                        }
                        Err(e) => self.set_status(format!("autostart: {e}"), false),
                    }
                }
                if ui.button("States").on_hover_text("explore what each plugin offers, with live values").clicked() {
                    self.states_ui.open = !self.states_ui.open;
                }
                if ui.button("Agent settings").clicked() {
                    self.show_settings = !self.show_settings;
                }
                ui.separator();
                if ui.button("Open file").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("json", &["json"]).pick_file() {
                        match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<Config>(&b).map_err(|e| e.to_string())) {
                            Ok(c) => {
                                self.cfg = c;
                                self.page = self.cfg.home.clone();
                                self.collect_icons();
                                self.dirty = true;
                                self.set_status(format!("opened {}", p.display()), true);
                            }
                            Err(e) => self.set_status(format!("open failed: {e}"), false),
                        }
                    }
                }
                if ui.button("Save file").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("json", &["json"]).set_file_name("deck-config.json").save_file() {
                        match serde_json::to_vec_pretty(&self.cfg) {
                            Ok(j) => {
                                let _ = std::fs::write(&p, j);
                                self.set_status(format!("saved {}", p.display()), true);
                            }
                            Err(e) => self.set_status(format!("{e}"), false),
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Firmware...").clicked() {
                        self.show_fw = !self.show_fw;
                    }
                    if ui.button("ESP32 bridge...").on_hover_text("optional hardware mod: run network plugins on the deck without the PC").clicked() {
                        self.show_bridge = !self.show_bridge;
                        if self.show_bridge && self.connected && self.cfg.agents.uart.enabled {
                            ui_bridge::refresh_status(self);
                        }
                    }
                    if ui.button("Log").clicked() {
                        self.fetch_log();
                    }
                    if ui.button(if self.calibrating { "Rings off" } else { "Rings" }).on_hover_text("tile centering test pattern on the deck").clicked() {
                        self.toggle_calibrate();
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).color(if self.status_ok { Color32::LIGHT_GREEN } else { Color32::from_gray(190) }));
                if !self.device_info.is_empty() {
                    ui.label(RichText::new(format!("  {}", short_info(&self.device_info))).weak().small());
                }
            });
            ui.add_space(2.0);
        });

        egui::SidePanel::left("pages").exact_width(left_w).show(ctx, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("Pages").strong());
            let order = self.cfg.order();
            for p in &order {
                if ui.selectable_label(self.page == *p, p).clicked() {
                    self.page = p.clone();
                    self.sel = Sel::None;
                }
            }
            ui.horizontal(|ui| {
                if ui.small_button("+ page").clicked() {
                    let mut n = 1;
                    while self.cfg.pages.contains_key(&format!("page{n}")) {
                        n += 1;
                    }
                    let name = format!("page{n}");
                    self.cfg.pages.insert(name.clone(), Page::default());
                    let mut o = self.cfg.order();
                    o.push(name.clone());
                    self.cfg.page_order = o;
                    self.page = name;
                    self.dirty = true;
                }
                if order.len() > 1 && ui.small_button("delete").clicked() {
                    let p = self.page.clone();
                    self.cfg.pages.remove(&p);
                    self.cfg.page_order.retain(|x| x != &p);
                    if self.cfg.home == p {
                        self.cfg.home = self.cfg.order().first().cloned().unwrap_or_default();
                    }
                    self.page = self.cfg.home.clone();
                    self.dirty = true;
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("rename");
                let mut name = self.page.clone();
                let resp = ui.add(egui::TextEdit::singleline(&mut name).desired_width(100.0));
                if resp.lost_focus() && name != self.page && !name.trim().is_empty() && !self.cfg.pages.contains_key(&name) {
                    if let Some(p) = self.cfg.pages.remove(&self.page) {
                        self.cfg.pages.insert(name.clone(), p);
                        let old = self.page.clone();
                        self.cfg.page_order = self.cfg.order().into_iter().map(|x| if x == old { name.clone() } else { x }).collect();
                        if self.cfg.home == old {
                            self.cfg.home = name.clone();
                        }
                        self.page = name;
                        self.dirty = true;
                    }
                }
            });
            ui.separator();
            ui.label(RichText::new("Deck").strong());
            if ui.selectable_label(self.sel == Sel::None, "Settings").clicked() {
                self.sel = Sel::None;
            }
            if ui.selectable_label(self.sel == Sel::SideLeft, "Left side button").clicked() {
                self.sel = Sel::SideLeft;
            }
            if ui.selectable_label(self.sel == Sel::SideRight, "Right side button").clicked() {
                self.sel = Sel::SideRight;
            }
            for k in 0..3 {
                if ui.selectable_label(self.sel == Sel::Knob(k), format!("Knob {}", k + 1)).clicked() {
                    self.sel = Sel::Knob(k);
                }
            }
            ui.add_space(8.0);
            if !self.pending_icons.is_empty() {
                ui.label(RichText::new(format!("{} icon(s) to upload on save", self.pending_icons.len())).weak().small());
            }
        });

        egui::SidePanel::right("inspector").exact_width(right_w).show(ctx, |ui| {
            ui.add_space(6.0);
            let w = ui.available_width();
            egui::ScrollArea::vertical().id_salt("inspector").auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(w - 12.0);
                ui_inspector::show(self, ui, ctx)
            });
        });

        egui::TopBottomPanel::bottom("library").exact_height(bottom_h).show(ctx, |ui| {
            ui.add_space(4.0);
            ui_library::show(self, ui);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui_canvas::show(self, ui, ctx);
        });

        ui_windows::show(self, ctx);
        ui_bridge::show(self, ctx);
    }
}

fn short_info(info: &str) -> String {
    let get = |k: &str| info.split(&format!("\"{k}\":\"")).nth(1).and_then(|r| r.split('"').next()).unwrap_or("").to_string();
    format!("{} {} | serial {}", get("firmware"), get("Dversion"), get("SerialNumber"))
}

/// Start the agent without a console window.
fn spawn_hidden(exe: &std::path::Path) -> std::io::Result<std::process::Child> {
    let mut c = std::process::Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.spawn()
}

fn start_agent(app: &mut App) {
    match agent_exe() {
        Some(p) => {
            match spawn_hidden(&p) {
                Ok(_) => {
                    app.set_status("agent started", true);
                    app.agent_running.store(true, Ordering::Relaxed);
                }
                Err(e) => app.set_status(format!("agent start failed: {e}"), false),
            }
        }
        None => app.set_status("d200x-agent.exe not found next to the configurator", false),
    }
}

/// The agent reads agent.json only at start: restart it so saved settings take effect.
/// `done` says what was saved, for the status line.
pub(crate) fn restart_agent(app: &mut App, done: &str) {
    if app.agent_running.load(Ordering::Relaxed) {
        stop_agent(app);
        start_agent(app);
        app.set_status(format!("{done}, agent restarted"), true);
    } else {
        app.set_status(format!("{done} (start the agent to activate)"), true);
    }
}

fn stop_agent(app: &mut App) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill").args(["/IM", "d200x-agent.exe", "/F"]).creation_flags(0x0800_0000).output();
    }
    app.agent_running.store(false, Ordering::Relaxed);
    app.set_status("agent stopped", true);
}
