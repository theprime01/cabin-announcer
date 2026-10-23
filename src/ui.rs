//! The window: live status, flight details, announcement scripts and voices.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::announcements::{SCRIPTS, script};
use crate::announcer;
use crate::audio::Audio;
use crate::config::{Config, packs_dir, sounds_dir};
use crate::packs::{self, Pack};
use crate::phase::{Cue, DoorState};
use crate::pmdg::{self, Model};
use crate::sim::{Command, Connection, Sim};
use crate::simbrief;

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Flight,
    Announcements,
    Packs,
    Voice,
    Log,
}

pub struct App {
    shared: Arc<Mutex<Config>>,
    cfg: Config,
    dirty_since: Option<Instant>,
    audio: Audio,
    sim: Sim,
    tab: Tab,
    message: Option<(String, bool)>,
    pmdg_seen_at: Option<Instant>,
    packs: Vec<Pack>,
    packs_read: Option<Instant>,
    install_source: String,
    install_airline: String,
    installing: Option<Receiver<Result<packs::Report, String>>>,
    confirm_remove: Option<String>,
    /// Pack previewed on the Announcements tab; None follows the pack in use.
    preview_pack: Option<String>,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, shared: Arc<Mutex<Config>>, audio: Audio, sim: Sim) -> App {
        let cfg = shared.lock().unwrap().clone();
        App {
            shared,
            cfg,
            dirty_since: None,
            audio,
            sim,
            tab: Tab::Flight,
            message: None,
            pmdg_seen_at: None,
            packs: Vec::new(),
            packs_read: None,
            install_source: String::new(),
            install_airline: String::new(),
            installing: None,
            confirm_remove: None,
            preview_pack: None,
        }
    }

    /// Re-reads the packs folder every few seconds, so packs you copy in by hand show up.
    fn refresh_packs(&mut self, force: bool) {
        if force || self.packs_read.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            self.packs = packs::list();
            self.packs_read = Some(Instant::now());
        }
    }

    fn detected_icao(&self) -> Option<String> {
        self.sim.state.lock().unwrap().live.airline.as_ref().map(|a| a.icao.clone())
    }

    fn active_pack(&self) -> Option<Pack> {
        packs::active(&self.cfg.voice.pack, self.detected_icao().as_deref())
    }

    fn start_install(&mut self, source: String) {
        if source.trim().is_empty() || self.installing.is_some() {
            return;
        }
        let airline = self.install_airline.trim().to_string();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(packs::install(&source, &airline));
        });
        self.installing = Some(rx);
        self.info("Installing…");
    }

    fn poll_install(&mut self) {
        let Some(rx) = &self.installing else { return };
        match rx.try_recv() {
            Ok(Ok(r)) => {
                self.installing = None;
                self.install_source.clear();
                let unsorted = if r.unsorted > 0 {
                    format!(" {} file(s) couldn't be matched to an announcement; they're in the pack's _unsorted folder.", r.unsorted)
                } else {
                    String::new()
                };
                self.info(format!("Installed \"{}\": {} recording(s) sorted.{unsorted}", r.id, r.sorted));
                self.refresh_packs(true);
            }
            Ok(Err(e)) => {
                self.installing = None;
                self.error(format!("Couldn't install: {e}"));
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.installing = None,
        }
    }

    fn info(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), false));
    }

    fn error(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), true));
    }

    /// Pushes edits to the sim thread at once, and saves to disk a second later.
    fn sync_config(&mut self) {
        let mut shared = self.shared.lock().unwrap();
        if *shared != self.cfg {
            *shared = self.cfg.clone();
            self.dirty_since = Some(Instant::now());
        }
        drop(shared);
        if self.dirty_since.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
            self.dirty_since = None;
            if let Err(e) = self.cfg.save() {
                self.error(format!("Couldn't save settings: {e}"));
            }
        }
    }

    fn test_play(&self, cue: Cue) {
        let live = self.sim.state.lock().unwrap().live.clone();
        if let Some(item) = announcer::item(cue, &self.cfg, &live, true) {
            self.audio.play(item);
        }
    }

    /// Plays these recordings, or the computer voice when empty.
    fn preview(&self, cue: Cue, recordings: Vec<std::path::PathBuf>) {
        let live = self.sim.state.lock().unwrap().live.clone();
        if let Some(mut item) = announcer::item(cue, &self.cfg, &live, true) {
            item.recordings = recordings;
            self.audio.play(item);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let s = self.sim.state.lock().unwrap().clone();
        let a = self.audio.status.lock().unwrap().clone();
        ui.horizontal(|ui| {
            match &s.connection {
                Connection::Connected => ui.colored_label(Color32::from_rgb(90, 200, 120), "Connected to MSFS"),
                Connection::NotRunning => ui.colored_label(Color32::GRAY, "Waiting for MSFS 2024…"),
                Connection::Error(e) => ui.colored_label(Color32::from_rgb(230, 90, 80), e.clone()),
            };
            if s.connection == Connection::Connected {
                ui.separator();
                ui.label(RichText::new(s.phase.label()).strong());
                if s.paused {
                    ui.label("(paused)");
                }
            }
        });
        if s.connection == Connection::Connected && !s.aircraft.is_empty() {
            ui.horizontal(|ui| {
                ui.label(&s.aircraft);
                match s.pmdg.as_ref().map(|p| p.model) {
                    Some(Model::B737) => ui.label(RichText::new("PMDG 737 data").color(Color32::from_rgb(90, 200, 120))),
                    Some(Model::B777) => ui.label(RichText::new("PMDG 777 data").color(Color32::from_rgb(90, 200, 120))),
                    None => ui.label(RichText::new("generic sim data").color(Color32::GRAY)),
                };
                ui.separator();
                match &s.live.airline {
                    Some(a) => ui.label(format!("Airline: {} (from the {})", if a.name.is_empty() { &a.icao } else { &a.name }, a.source)),
                    None => ui.label(RichText::new("Airline not recognised").color(Color32::GRAY)),
                };
                if let Some(p) = self.active_pack() {
                    ui.separator();
                    ui.label(format!("Sound pack: {}", p.title()));
                }
            });
        }
        // PMDG aircraft loaded but no SDK data after a while: the broadcast is off.
        let is_pmdg = s.aircraft.to_ascii_uppercase().contains("PMDG");
        if is_pmdg && s.pmdg.is_none() {
            let since = *self.pmdg_seen_at.get_or_insert_with(Instant::now);
            if since.elapsed() > Duration::from_secs(15) {
                ui.horizontal(|ui| {
                    ui.colored_label(Color32::from_rgb(230, 180, 60), "No PMDG data yet. The SDK broadcast may be switched off.");
                    if ui.button("Switch it on").clicked() {
                        match pmdg::enable_broadcast() {
                            Ok(f) if f.is_empty() => self.info("The broadcast is already on. Try reloading the aircraft."),
                            Ok(f) => self.info(format!("Switched on in {} file(s). Reload the aircraft (or restart the flight).", f.len())),
                            Err(e) => self.error(e),
                        }
                    }
                });
            }
        } else {
            self.pmdg_seen_at = None;
        }
        ui.horizontal(|ui| {
            match &a.now_playing {
                Some(t) => ui.label(format!("Now playing: {t}")),
                None => ui.label(RichText::new("Nothing playing").color(Color32::GRAY)),
            };
            if !a.queued.is_empty() {
                ui.label(RichText::new(format!("then {}", a.queued.join(", "))).color(Color32::GRAY));
            }
            if (a.now_playing.is_some() || !a.queued.is_empty()) && ui.button("Stop").clicked() {
                self.audio.stop();
            }
        });
        if let Some(e) = &a.error {
            ui.colored_label(Color32::from_rgb(230, 90, 80), e);
        }
        if let Some((m, is_err)) = self.message.clone() {
            ui.horizontal(|ui| {
                let color = if is_err { Color32::from_rgb(230, 90, 80) } else { Color32::from_rgb(120, 170, 230) };
                ui.colored_label(color, m);
                if ui.small_button("✕").clicked() {
                    self.message = None;
                }
            });
        }
    }

    fn flight_tab(&mut self, ui: &mut egui::Ui) {
        let s = self.sim.state.lock().unwrap().clone();
        ui.heading("This flight");
        ui.horizontal(|ui| {
            ui.label("SimBrief username or pilot ID");
            ui.add(egui::TextEdit::singleline(&mut self.cfg.simbrief_user).desired_width(160.0));
            if ui.button("Load latest flight plan").clicked() {
                match simbrief::fetch(&self.cfg.simbrief_user, &self.cfg.flight.airline, &self.cfg.flight.captain) {
                    Ok(f) => {
                        let msg = format!("Loaded {} to {}", f.flight_number, f.destination);
                        self.cfg.flight = f;
                        self.info(msg);
                    }
                    Err(e) => self.error(e),
                }
            }
        });
        ui.add_space(6.0);
        egui::Grid::new("flight").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            let f = &mut self.cfg.flight;
            ui.label("Airline");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut f.airline);
                if f.airline.is_empty() {
                    let hint = match &s.live.airline {
                        Some(a) if !a.name.is_empty() => format!("detected: {}", a.name),
                        _ => "detected from your livery or flight plan".to_string(),
                    };
                    ui.label(RichText::new(hint).color(Color32::GRAY));
                }
            });
            ui.end_row();
            ui.label("Airline code (ICAO)");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut f.airline_icao).desired_width(60.0));
                ui.label(RichText::new("e.g. NAX for Norwegian; picks the matching sound pack").color(Color32::GRAY));
            });
            ui.end_row();
            ui.label("Flight number");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut f.flight_number);
                if f.flight_number.is_empty() {
                    if let Some(p) = s.pmdg.as_ref().filter(|p| !p.flight_number.is_empty()) {
                        ui.label(RichText::new(format!("FMC: {}", p.flight_number)).color(Color32::GRAY));
                    }
                }
            });
            ui.end_row();
            ui.label("From");
            ui.text_edit_singleline(&mut f.origin);
            ui.end_row();
            ui.label("To");
            ui.text_edit_singleline(&mut f.destination);
            ui.end_row();
            ui.label("Captain's name");
            ui.text_edit_singleline(&mut f.captain);
            ui.end_row();
            ui.label("Flight time (minutes)");
            optional_number(ui, &mut f.flight_time_minutes, 10);
            ui.end_row();
            ui.label("Cruise altitude (feet)");
            optional_number(ui, &mut f.cruise_altitude_ft, 1000);
            ui.end_row();
        });
        ui.label(RichText::new("Leave anything blank and the announcements simply leave it out.").color(Color32::GRAY));

        ui.add_space(12.0);
        ui.heading("What the sim says");
        if s.connection != Connection::Connected {
            ui.label("Start MSFS 2024 and this fills in on its own.");
        } else {
            let t = &s.telemetry;
            egui::Grid::new("telemetry").num_columns(4).spacing([16.0, 4.0]).show(ui, |ui| {
                ui.label("Phase");
                ui.label(RichText::new(s.phase.label()).strong());
                ui.label("Seat belt sign");
                ui.label(if t.belts { "ON" } else { "off" });
                ui.end_row();
                ui.label("Altitude");
                ui.label(format!("{:.0} ft ({:.0} AGL)", t.altitude, t.agl));
                ui.label("Doors");
                ui.label(match t.doors {
                    DoorState::Open => "open",
                    DoorState::Closed => "closed",
                    DoorState::Armed => "armed",
                    DoorState::Unknown => "unknown",
                });
                ui.end_row();
                ui.label("Ground speed");
                ui.label(format!("{:.0} kt", t.ground_speed));
                ui.label("Engines");
                ui.label(if t.engines_running { "running" } else { "off" });
                ui.end_row();
                ui.label("Vertical speed");
                ui.label(format!("{:.0} fpm", t.vertical_speed));
                ui.label("Parking brake");
                ui.label(if t.parking_brake { "set" } else { "off" });
                ui.end_row();
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("New flight (back to boarding)").clicked() {
                self.sim.send(Command::NewFlight);
            }
            ui.label(RichText::new("Use this for a turnaround without reloading the flight.").color(Color32::GRAY));
        });
    }

    fn announcements_tab(&mut self, ui: &mut egui::Ui) {
        let played = self.sim.state.lock().unwrap().played.clone();
        self.refresh_packs(false);
        let in_use = self.active_pack();
        // The pack being previewed: the one in use now, unless you pick another.
        let pack = match &self.preview_pack {
            Some(id) => self.packs.iter().find(|p| &p.id == id).cloned(),
            None => in_use.clone(),
        };
        ui.horizontal(|ui| {
            ui.label("These play by themselves as the flight goes on. You can switch each one off, edit its words, or play it now.");
        });
        ui.horizontal(|ui| {
            ui.label("Recordings from");
            let in_use_label = match &in_use {
                Some(p) => format!("The pack in use now ({})", p.title()),
                None => "The pack in use now (none)".to_string(),
            };
            let shown = match &self.preview_pack {
                None => in_use_label.clone(),
                Some(_) => pack.as_ref().map_or("(removed pack)".to_string(), |p| p.title().to_string()),
            };
            egui::ComboBox::from_id_salt("preview_pack").selected_text(shown).width(280.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.preview_pack, None, in_use_label);
                for p in &self.packs {
                    ui.selectable_value(&mut self.preview_pack, Some(p.id.clone()), p.title());
                }
            });
            if self.packs.is_empty() {
                ui.label(RichText::new("Install a pack on the Sound packs tab to preview real announcements.").color(Color32::GRAY));
            } else if self.preview_pack.is_some() {
                ui.label(RichText::new("Previewing only; flights still use the pack in use now.").color(Color32::GRAY));
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Placeholders: {airline} {flight} {origin} {destination} {captain} {flight_time} {cruise_altitude} {time_remaining} {local_time} {temperature} {daypart}. A [bracketed part] is left out when its details are unknown.").small().color(Color32::GRAY));
        });
        ui.horizontal(|ui| {
            if ui.button("Open my recordings folder").clicked() {
                let dir = sounds_dir();
                for s in SCRIPTS {
                    let _ = std::fs::create_dir_all(dir.join(s.key));
                }
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
            ui.label(RichText::new("Drop .wav/.mp3/.ogg files into an announcement's folder to use your own voice instead.").small().color(Color32::GRAY));
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for s in SCRIPTS {
                let mut settings = self.cfg.announcement(s.key);
                let before = settings.clone();
                let versions = packs::versions(s.key, pack.as_ref());
                ui.push_id(s.key, |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut settings.enabled, "");
                        let done = if played.contains(&s.cue) { " ✔" } else { "" };
                        ui.label(RichText::new(format!("{}{done}", s.title)).strong());
                        ui.label(RichText::new(s.role.label()).color(Color32::GRAY));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("▶ Voice").on_hover_text("The computer voice reading the words below").clicked() {
                                self.preview(s.cue, Vec::new());
                            }
                            if !versions.files.is_empty() {
                                let tip = format!("From {}", versions.from);
                                if ui.button("▶ Recording").on_hover_text(tip).clicked() {
                                    self.preview(s.cue, packs::recordings(s.key, pack.as_ref()));
                                }
                            }
                        });
                    });
                    if versions.files.len() > 1 {
                        let heading = format!("{} recorded versions from {}", versions.files.len(), versions.from);
                        ui.collapsing(heading, |ui| {
                            for f in &versions.files {
                                ui.horizontal(|ui| {
                                    if ui.small_button("▶").clicked() {
                                        self.preview(s.cue, vec![f.clone()]);
                                    }
                                    ui.label(f.file_name().unwrap_or_default().to_string_lossy());
                                });
                            }
                        });
                    } else if let Some(f) = versions.files.first() {
                        ui.label(
                            RichText::new(format!("Recording from {}: {}", versions.from, f.file_name().unwrap_or_default().to_string_lossy()))
                                .small()
                                .color(Color32::from_rgb(120, 170, 230)),
                        );
                    }
                    ui.collapsing("Words", |ui| {
                        let mut text = settings.text.clone().unwrap_or_else(|| script(s.cue).text.to_string());
                        let resp = ui.add(egui::TextEdit::multiline(&mut text).desired_rows(3).desired_width(f32::INFINITY));
                        if resp.changed() {
                            settings.text = (text.trim() != s.text).then_some(text);
                        }
                        if settings.text.is_some() && ui.small_button("Back to the original words").clicked() {
                            settings.text = None;
                        }
                    });
                });
                if settings != before {
                    self.cfg.announcements.insert(s.key.to_string(), settings);
                }
                ui.separator();
            }
        });
    }

    fn packs_tab(&mut self, ui: &mut egui::Ui) {
        self.refresh_packs(false);
        let detected = self.sim.state.lock().unwrap().live.airline.clone();
        let active = self.active_pack();
        let gray = |t: &str| RichText::new(t).color(Color32::GRAY);

        ui.label(
            "A sound pack is a set of real recorded announcements. When one is installed for your airline, \
             it plays instead of the computer voice. Announcements the pack doesn't cover still use the voice.",
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Use");
            let shown = match self.cfg.voice.pack.as_str() {
                "" => "Automatic (match the airline)".to_string(),
                "none" => "No pack (computer voice only)".to_string(),
                id => self.packs.iter().find(|p| p.id == id).map_or(id.to_string(), |p| p.title().to_string()),
            };
            egui::ComboBox::from_id_salt("pack").selected_text(shown).width(260.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.cfg.voice.pack, String::new(), "Automatic (match the airline)");
                ui.selectable_value(&mut self.cfg.voice.pack, "none".to_string(), "No pack (computer voice only)");
                for p in &self.packs {
                    ui.selectable_value(&mut self.cfg.voice.pack, p.id.clone(), p.title());
                }
            });
            let now = match (&active, &detected) {
                (Some(p), _) => format!("now playing from: {}", p.title()),
                (None, Some(a)) => format!("no pack for {} yet", if a.name.is_empty() { &a.icao } else { &a.name }),
                (None, None) => "airline not recognised yet".to_string(),
            };
            ui.label(gray(&now));
        });

        ui.add_space(10.0);
        ui.heading("Install a pack");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.install_source)
                    .hint_text("zip file, folder, or https:// download link")
                    .desired_width(380.0),
            );
            ui.label("for airline");
            let hint = detected.as_ref().map_or("NAX".to_string(), |a| a.icao.clone());
            ui.add(egui::TextEdit::singleline(&mut self.install_airline).hint_text(hint).desired_width(50.0));
            let busy = self.installing.is_some();
            if ui.add_enabled(!busy, egui::Button::new(if busy { "Installing…" } else { "Install" })).clicked() {
                let source = self.install_source.clone();
                self.start_install(source);
            }
        });
        ui.label(gray(
            "Or drag a zip or folder onto this window. Files are sorted into announcements by their names \
             (\"safety\", \"boarding\", \"turbulence\"…). Leave the airline blank for a pack that plays on every airline.",
        ));
        ui.label(gray(
            "Only install recordings you have the right to use, such as packs you've bought, packs shared with \
             permission, or your own. Real airline announcements are copyrighted.",
        ));

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.heading("Installed");
            if ui.small_button("Open packs folder").clicked() {
                let _ = std::fs::create_dir_all(packs_dir());
                let _ = std::process::Command::new("explorer").arg(packs_dir()).spawn();
            }
        });
        if self.packs.is_empty() {
            ui.label(gray("No packs yet."));
        }
        let mut changed: Option<(String, packs::Manifest)> = None;
        let mut remove: Option<String> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for p in &self.packs {
                ui.push_id(&p.id, |ui| {
                    ui.horizontal(|ui| {
                        let mark = if active.as_ref().is_some_and(|a| a.id == p.id) { "▶ " } else { "" };
                        ui.label(RichText::new(format!("{mark}{}", p.title())).strong());
                        ui.label(gray(&format!("{} of {} announcements", p.covers.len(), SCRIPTS.len())));
                        if p.unsorted > 0 {
                            ui.colored_label(Color32::from_rgb(230, 180, 60), format!("{} unsorted", p.unsorted));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let confirming = self.confirm_remove.as_deref() == Some(p.id.as_str());
                            if ui.button(if confirming { "Really remove?" } else { "Remove" }).clicked() {
                                if confirming { remove = Some(p.id.clone()) } else { self.confirm_remove = Some(p.id.clone()) }
                            }
                            if ui.button("Open folder").clicked() {
                                let _ = std::process::Command::new("explorer").arg(p.dir()).spawn();
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        let mut m = p.manifest.clone();
                        let mut codes = m.airlines.join(", ");
                        ui.label("Airlines");
                        let r = ui.add(egui::TextEdit::singleline(&mut codes).hint_text("all").desired_width(140.0));
                        if r.lost_focus() && codes != m.airlines.join(", ") {
                            m.airlines = codes.split([',', ' ']).filter(|c| !c.is_empty()).map(|c| c.to_ascii_uppercase()).collect();
                            changed = Some((p.id.clone(), m.clone()));
                        }
                        if ui.checkbox(&mut m.play_all, "Play every file in order (e.g. two languages)").changed() {
                            changed = Some((p.id.clone(), m));
                        }
                    });
                    let missing: Vec<&str> = SCRIPTS.iter().filter(|s| !p.covers.contains(&s.key)).map(|s| s.title).collect();
                    if !missing.is_empty() {
                        ui.label(RichText::new(format!("Computer voice for: {}", missing.join(", "))).small().color(Color32::GRAY));
                    }
                });
                ui.separator();
            }
        });
        if let Some((id, m)) = changed {
            match packs::save_manifest(&id, &m) {
                Ok(()) => self.refresh_packs(true),
                Err(e) => self.error(e),
            }
        }
        if let Some(id) = remove {
            self.confirm_remove = None;
            match packs::remove(&id) {
                Ok(()) => {
                    if self.cfg.voice.pack == id {
                        self.cfg.voice.pack.clear();
                    }
                    self.info(format!("Removed {id}"));
                    self.refresh_packs(true);
                }
                Err(e) => self.error(e),
            }
        }
    }

    fn voice_tab(&mut self, ui: &mut egui::Ui) {
        let voices = self.audio.status.lock().unwrap().voices.clone();
        let v = &mut self.cfg.voice;
        egui::Grid::new("voice").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            for (label, value) in [("Captain's voice", &mut v.captain_voice), ("Cabin crew voice", &mut v.purser_voice)] {
                ui.label(label);
                let shown = if value.is_empty() { "Automatic".to_string() } else { value.clone() };
                egui::ComboBox::from_id_salt(label).selected_text(shown).width(280.0).show_ui(ui, |ui| {
                    ui.selectable_value(value, String::new(), "Automatic");
                    for name in &voices {
                        ui.selectable_value(value, name.clone(), name);
                    }
                });
                ui.end_row();
            }
            ui.label("Speaking speed");
            ui.add(egui::Slider::new(&mut v.rate, 0.7..=1.3));
            ui.end_row();
            ui.label("Volume");
            ui.add(egui::Slider::new(&mut v.volume, 0.0..=1.0));
            ui.end_row();
            ui.label("Cabin PA sound");
            ui.checkbox(&mut v.pa_effect, "Make speech sound like it's coming through the cabin speakers");
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut v.pa_effect_on_recordings, "Apply it to my own recordings too");
            ui.end_row();
            ui.label("Chime");
            ui.checkbox(&mut v.chime, "Play the PA chime before announcements");
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("▶ Try the captain").clicked() {
                self.test_play(Cue::CrewSeatsTakeoff);
            }
            if ui.button("▶ Try the cabin crew").clicked() {
                self.test_play(Cue::Turbulence);
            }
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "More voices: Windows Settings > Time & language > Speech > Add voices. \
                 Restart Cabin Announcer after adding one.",
            )
            .color(Color32::GRAY),
        );
    }

    fn log_tab(&mut self, ui: &mut egui::Ui) {
        let log = self.sim.state.lock().unwrap().log.clone();
        egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
            for l in &log {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&l.time).monospace().color(Color32::GRAY));
                    ui.label(&l.text);
                });
            }
        });
    }
}

fn optional_number(ui: &mut egui::Ui, value: &mut Option<u32>, step: u32) {
    let mut n = value.unwrap_or(0);
    if ui.add(egui::DragValue::new(&mut n).speed(step as f64).range(0..=60_000)).changed() {
        *value = (n > 0).then_some(n);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("status").show(ctx, |ui| {
            ui.add_space(4.0);
            self.status_bar(ui);
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (Tab::Flight, "Flight"),
                    (Tab::Announcements, "Announcements"),
                    (Tab::Packs, "Sound packs"),
                    (Tab::Voice, "Voices"),
                    (Tab::Log, "Log"),
                ] {
                    ui.selectable_value(&mut self.tab, tab, label);
                }
            });
            ui.add_space(2.0);
        });
        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Flight => self.flight_tab(ui),
            Tab::Announcements => self.announcements_tab(ui),
            Tab::Packs => self.packs_tab(ui),
            Tab::Voice => self.voice_tab(ui),
            Tab::Log => self.log_tab(ui),
        });
        // Dropping a zip or folder anywhere on the window installs it.
        let dropped: Vec<String> = ctx.input(|i| {
            i.raw.dropped_files.iter().filter_map(|f| f.path.as_ref().map(|p| p.display().to_string())).collect()
        });
        if let Some(first) = dropped.into_iter().next() {
            self.tab = Tab::Packs;
            self.start_install(first);
        }
        self.poll_install();
        self.sync_config();
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.cfg.save();
    }
}
