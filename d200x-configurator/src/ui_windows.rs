//! Floating windows: agent settings, Material icon picker, firmware, deck log.

use crate::app::{color_to_string, repo_root, App, MdiTarget, Sel};
use crate::ha;
use crate::ui_states::{self, KeyTarget, StateField, Use};
use eframe::egui::{self, RichText};
use std::sync::atomic::Ordering;

pub fn show(app: &mut App, ctx: &egui::Context) {
    settings(app, ctx);
    mdi(app, ctx);
    minecraft(app, ctx);
    firmware(app, ctx);
    log(app, ctx);
    states(app, ctx);
}

fn states(app: &mut App, ctx: &egui::Context) {
    app.states_ui.bridge_enabled = app.connected && app.cfg.agents.uart.enabled;
    let target = match app.sel {
        Sel::Knob(_) => app.selected_knob().map(|k| KeyTarget { label: None, state: None, has_ring: k.progress.is_some() }),
        _ => app.selected_key().map(|k| KeyTarget { label: Some(k.label.clone()), state: k.state.clone(), has_ring: k.progress.is_some() }),
    }
    .unwrap_or_default();
    let Some(used) = ui_states::window(ctx, &mut app.states_ui, &target) else { return };
    // a knob only has an indicator
    if let Some(knob) = app.selected_knob() {
        match used {
            Use::Set(StateField::Ring, s) => match knob.progress.as_mut() {
                Some(p) => p.state = s,
                None => knob.progress = Some(deck_config::Progress { max: crate::states::ring_max(&s), state: s, style: "fill".into(), start: "left".into(), ..Default::default() }),
            },
            Use::Set(StateField::RingColor, s) => {
                if let Some(p) = knob.progress.as_mut() {
                    p.color_state = Some(s);
                }
            }
            _ => return,
        }
        app.dirty = true;
        return;
    }
    let Some(key) = app.selected_key() else { return };
    match used {
        Use::Set(StateField::KeyState, s) => key.state = Some(s),
        Use::Set(StateField::Label, s) => key.value = Some(s),
        Use::Set(StateField::Background, s) => key.color_state = Some(s),
        Use::Set(StateField::RingColor, s) => {
            if let Some(p) = key.progress.as_mut() {
                p.color_state = Some(s);
            }
        }
        Use::Set(StateField::Ring, s) => match key.progress.as_mut() {
            Some(p) => p.state = s,
            None => key.progress = Some(deck_config::Progress { max: crate::states::ring_max(&s), state: s, min: 0.0, style: "border".into(), start: "top".into(), color: "#f5b400".into(), width: 6, color_state: None, ..Default::default() }),
        },
        Use::Look(v) => {
            key.looks.entry(v).or_default();
        }
    }
    app.dirty = true;
}

fn settings(app: &mut App, ctx: &egui::Context) {
    if !app.show_settings {
        return;
    }
    let mut open = app.show_settings;
    egui::Window::new("Agent settings").open(&mut open).default_width(480.0).show(ctx, |ui| {
        ui.label(RichText::new("Home Assistant").strong());
        ui.horizontal(|ui| {
            ui.label("URL");
            ui.add(egui::TextEdit::singleline(&mut app.agent_cfg.homeassistant.url).desired_width(320.0).hint_text("https://ha.example.com"));
        });
        ui.horizontal(|ui| {
            ui.label("Token");
            ui.add(egui::TextEdit::singleline(&mut app.agent_cfg.homeassistant.token).password(true).desired_width(320.0));
        });
        ui.small("Long-lived access token (HA: profile > Security). Stays on this PC; the deck never sees it.");
        ui.separator();
        ui.label(RichText::new("OBS, Spotify and Discord are set up in their library tabs.").weak());
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                match ha::save_agent_config(&app.agent_cfg) {
                    Ok(()) => crate::restart_agent(app, &format!("saved {}", ha::agent_config_path().display())),
                    Err(e) => app.set_status(format!("{e}"), false),
                }
            }
            if ui.button("Test Home Assistant").clicked() {
                match ha::fetch_entities(&app.agent_cfg.homeassistant) {
                    Ok(v) => {
                        let n = v.len();
                        app.ha_entities = v;
                        app.set_status(format!("Home Assistant OK: {n} entities"), true);
                    }
                    Err(e) => app.set_status(format!("{e}"), false),
                }
            }
        });
        ui.small("Restart the agent after changing settings (Agent: stop / start in the top bar).");
    });
    app.show_settings = open;
}

fn mdi(app: &mut App, ctx: &egui::Context) {
    if !app.show_mdi {
        return;
    }
    let mut open = app.show_mdi;
    let mut chosen: Option<String> = None;
    egui::Window::new("Material Design Icons").open(&mut open).default_size([560.0, 460.0]).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("search");
            ui.add(egui::TextEdit::singleline(&mut app.mdi_filter).desired_width(200.0));
            ui.label("colour");
            crate::ui_color::color_field(ui, &mut app.mdi_color);
            ui.label(RichText::new(format!("{} icons", app.mdi_names.len())).weak());
        });
        let f = app.mdi_filter.to_lowercase();
        let color = color_to_string(app.mdi_color);
        let names: Vec<String> = app.mdi_names.iter().filter(|n| f.is_empty() || n.contains(&f)).take(160).cloned().collect();
        ui.small(if names.len() >= 160 { "showing the first 160 matches; refine the search" } else { "" });
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for n in names {
                    let resp = match app.mdi_thumb(ctx, &n, &color) {
                        Some(tex) => ui.add(egui::ImageButton::new(egui::Image::new(&tex).fit_to_exact_size(egui::Vec2::splat(36.0)))).on_hover_text(&n),
                        None => ui.small_button(&n),
                    };
                    if resp.clicked() {
                        chosen = Some(n.clone());
                    }
                }
            });
        });
        if app.mdi_names.is_empty() {
            ui.label("Icon set not found: expected assets/mdi in the project folder.");
        }
    });
    app.show_mdi = open;
    if let Some(n) = chosen {
        let color = color_to_string(app.mdi_color);
        if let Some(file) = app.mdi_icon(&n, &color) {
            apply_picked_icon(app, &file);
            app.show_mdi = false;
        }
    }
}

/// Apply a rendered icon file to the picker's target (key icon or a look's icon).
fn apply_picked_icon(app: &mut App, file: &str) {
    let target = app.mdi_target.clone();
    if let Sel::Key(_) = app.sel {
        if let Some(key) = app.selected_key() {
            match target {
                MdiTarget::LookIcon(rule) => {
                    if let Some(l) = key.looks.get_mut(&rule) {
                        l.icon = Some(file.to_string());
                    }
                }
                _ => key.icon = Some(file.to_string()),
            }
        }
    }
    app.dirty = true;
    app.set_status(format!("icon {file} ready (uploaded on save)"), true);
}

fn minecraft(app: &mut App, ctx: &egui::Context) {
    if !app.show_mc {
        return;
    }
    let mut open = app.show_mc;
    let mut chosen: Option<String> = None;
    let mut import = false;
    let mut import_pick = false;
    egui::Window::new("Minecraft items").open(&mut open).default_size([600.0, 480.0]).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("search");
            ui.add(egui::TextEdit::singleline(&mut app.mc_filter).desired_width(200.0).hint_text("diamond, sword, potion..."));
            ui.label(RichText::new(format!("{} items", app.mc_names.len())).weak());
            if ui.small_button("Re-import").on_hover_text("read the item textures again from the installed Minecraft client").clicked() {
                import = true;
            }
        });
        if app.mc_names.is_empty() {
            ui.add_space(8.0);
            ui.label("No Minecraft item icons yet. They are read from the game's own client jar on this PC (nothing is downloaded).");
            ui.horizontal(|ui| {
                if ui.button("Import from the installed Minecraft").clicked() {
                    import = true;
                }
                if ui.button("Pick a client .jar...").clicked() {
                    import_pick = true;
                }
            });
            return;
        }
        let f = app.mc_filter.to_lowercase();
        let words: Vec<&str> = f.split_whitespace().collect();
        let names: Vec<String> = app.mc_names.iter().filter(|n| words.iter().all(|w| n.contains(w))).take(240).cloned().collect();
        ui.small(if names.len() >= 240 { "showing the first 240 matches; refine the search" } else { "pixel art is scaled 6x on the deck; pick one to use it as the icon" });
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for n in names {
                    let resp = match app.mc_thumb(ctx, &n) {
                        Some(tex) => ui.add(egui::ImageButton::new(egui::Image::new(&tex).fit_to_exact_size(egui::Vec2::splat(36.0)))).on_hover_text(n.replace('_', " ")),
                        None => ui.small_button(&n),
                    };
                    if resp.clicked() {
                        chosen = Some(n.clone());
                    }
                }
            });
        });
    });
    app.show_mc = open;
    if import {
        app.import_minecraft(None);
    }
    if import_pick {
        if let Some(p) = rfd::FileDialog::new().add_filter("Minecraft client", &["jar"]).pick_file() {
            app.import_minecraft(Some(p));
        }
    }
    if let Some(n) = chosen {
        if let Some(file) = app.mc_icon(&n) {
            apply_picked_icon(app, &file);
            app.show_mc = false;
        }
    }
}

fn firmware(app: &mut App, ctx: &egui::Context) {
    if !app.show_fw {
        return;
    }
    let mut open = app.show_fw;
    let running = app.fw_running.load(Ordering::SeqCst);
    egui::Window::new("Firmware").open(&mut open).default_size([720.0, 480.0]).show(ctx, |ui| {
        ui.label("Writes only the app partition through Ulanzi's own updater. Bootloader, kernel and recovery stay untouched; holding the reset key at power-on always restores the factory image.");
        ui.horizontal(|ui| {
            if ui.add_enabled(!running, egui::Button::new("Install deckd to flash")).clicked() {
                app.fw_confirm = Some("deckd");
            }
            if ui.add_enabled(!running, egui::Button::new("Restore Ulanzi firmware")).clicked() {
                app.fw_confirm = Some("stock");
            }
            if running {
                ui.spinner();
                ui.label("running: unplug and replug the deck when asked");
            }
        });
        if let Some(which) = app.fw_confirm {
            ui.separator();
            let what = if which == "stock" { "restore the original Ulanzi firmware" } else { "install deckd permanently" };
            ui.label(RichText::new(format!("This will {what}. Close Ulanzi Studio first. Continue?")).strong());
            ui.horizontal(|ui| {
                if ui.button("Yes, flash").clicked() {
                    app.fw_confirm = None;
                    run_flash(app, which);
                }
                if ui.button("Cancel").clicked() {
                    app.fw_confirm = None;
                }
            });
        }
        ui.separator();
        let text = app.fw_log.lock().unwrap().clone();
        egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut text.as_str()).desired_width(f32::INFINITY).desired_rows(16).font(egui::TextStyle::Monospace));
        });
    });
    app.show_fw = open;
    if running {
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

fn run_flash(app: &mut App, which: &str) {
    let Some(root) = repo_root() else {
        app.set_status("repository (research/adb-install.ps1) not found next to the configurator", false);
        return;
    };
    let image = if which == "stock" { root.join("research/stock-firmware/update_5.3.6_vendor.img") } else { root.join("d200x-device/image/out/update.img") };
    if !image.exists() {
        app.set_status(format!("image missing: {}", image.display()), false);
        return;
    }
    let script = root.join("research/adb-install.ps1");
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script).arg("-Image").arg(&image);
    if which != "stock" {
        if let Some(serial) = app.device_info.split("\"SerialNumber\":\"").nth(1).and_then(|r| r.split('"').next()) {
            if serial != "0123456789ABCDEF" {
                cmd.arg("-Serial").arg(serial);
            }
        }
    }
    cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    match cmd.spawn() {
        Ok(mut child) => {
            *app.fw_log.lock().unwrap() = format!("> {} -Image {}\n\nUNPLUG AND REPLUG THE DECK NOW.\n\n", script.display(), image.display());
            app.fw_running.store(true, Ordering::SeqCst);
            let log = app.fw_log.clone();
            let running = app.fw_running.clone();
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            // The reader threads may outlive the script: the adb server it starts inherits the
            // pipes and keeps them open. Completion therefore follows the process exit, not EOF.
            {
                use std::io::BufRead;
                let log2 = log.clone();
                if let Some(e) = stderr {
                    std::thread::spawn(move || {
                        for line in std::io::BufReader::new(e).lines().map_while(Result::ok) {
                            if !line.trim().is_empty() && !line.contains("CategoryInfo") && !line.contains("FullyQualifiedErrorId") && !line.starts_with("In ") && !line.starts_with('+') {
                                log2.lock().unwrap().push_str(&format!("{line}\n"));
                            }
                        }
                    });
                }
                let log3 = log.clone();
                if let Some(o) = stdout {
                    std::thread::spawn(move || {
                        for line in std::io::BufReader::new(o).lines().map_while(Result::ok) {
                            log3.lock().unwrap().push_str(&format!("{line}\n"));
                        }
                    });
                }
            }
            std::thread::spawn(move || {
                let status = child.wait();
                // give the readers a moment to flush the last lines
                std::thread::sleep(std::time::Duration::from_millis(300));
                let code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
                log.lock().unwrap().push_str(&format!("\n[finished{}]\n", if code == 0 { String::new() } else { format!(", exit code {code}") }));
                running.store(false, Ordering::SeqCst);
            });
            app.set_status(format!("firmware script started: {which}"), true);
        }
        Err(e) => app.set_status(format!("cannot start powershell: {e}"), false),
    }
}

fn log(app: &mut App, ctx: &egui::Context) {
    if !app.show_log {
        return;
    }
    let mut open = app.show_log;
    egui::Window::new("Deck log").open(&mut open).default_size([720.0, 420.0]).show(ctx, |ui| {
        if ui.button("Refresh").clicked() {
            app.fetch_log();
        }
        egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut app.log_text.as_str()).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
        });
    });
    app.show_log = open;
}
