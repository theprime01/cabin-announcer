//! The window: live status, flight details, announcements, packs and sound.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::announcements::{BOARDING_MUSIC, CABIN_AMBIENCE, SCRIPTS};
use crate::announcer;
use crate::audio::{Audio, Item, View};
use crate::autostart;
use crate::config::{Config, packs_dir};
use crate::hotkeys::{self, Hotkeys};
use crate::packs::{self, Pack};
use crate::phase::{Cue, DoorState, GSX_COMPLETED, GSX_PERFORMING, GSX_REQUESTED};
use crate::pmdg::{self, Model};
use crate::sim::{Command, Connection, Sim};
use crate::simbrief;

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Flight,
    Announcements,
    Packs,
    Sound,
    Log,
}

const GREEN: Color32 = Color32::from_rgb(90, 200, 120);
const RED: Color32 = Color32::from_rgb(230, 90, 80);
const AMBER: Color32 = Color32::from_rgb(230, 180, 60);
const BLUE: Color32 = Color32::from_rgb(120, 170, 230);

fn gray(t: impl Into<String>) -> RichText {
    RichText::new(t).color(Color32::GRAY)
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().to_string()
}

type Job<T> = Option<Receiver<Result<T, String>>>;

pub struct App {
    shared: Arc<Mutex<Config>>,
    cfg: Config,
    dirty_since: Option<Instant>,
    audio: Audio,
    sim: Sim,
    hotkeys: Hotkeys,
    hotkeys_failed: Arc<AtomicBool>,
    autostart: bool,
    tab: Tab,
    message: Option<(String, bool)>,
    pmdg_seen_at: Option<Instant>,
    packs: Vec<Pack>,
    packs_read: Option<Instant>,
    fenix_sets: usize,
    install_source: String,
    install_airline: String,
    installing: Job<packs::Report>,
    importing: Job<Vec<packs::Report>>,
    confirm_remove: Option<String>,
    /// Pack previewed on the Announcements tab; None follows the pack in use.
    preview_pack: Option<String>,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, shared: Arc<Mutex<Config>>, audio: Audio, sim: Sim, _autostarted: bool) -> App {
        let cfg = shared.lock().unwrap().clone();
        let failed = Arc::new(AtomicBool::new(false));
        let f = failed.clone();
        let hotkeys = Hotkeys::start(audio.clone(), cfg.hotkeys, move || f.store(true, Ordering::SeqCst));
        App {
            shared,
            cfg,
            dirty_since: None,
            audio,
            sim,
            hotkeys,
            hotkeys_failed: failed,
            autostart: autostart::is_enabled(),
            tab: Tab::Flight,
            message: None,
            pmdg_seen_at: None,
            packs: Vec::new(),
            packs_read: None,
            fenix_sets: packs::find_fenix_sets().len(),
            install_source: String::new(),
            install_airline: String::new(),
            installing: None,
            importing: None,
            confirm_remove: None,
            preview_pack: None,
        }
    }

    fn info(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), false));
    }

    fn error(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), true));
    }

    /// Pushes edits to the other threads at once, and saves to disk a second later.
    fn sync_config(&mut self) {
        let mut shared = self.shared.lock().unwrap();
        if *shared != self.cfg {
            if shared.sound != self.cfg.sound {
                self.audio.settings(self.cfg.sound.clone());
            }
            if shared.hotkeys != self.cfg.hotkeys {
                self.hotkeys_failed.store(false, Ordering::SeqCst);
                self.hotkeys.set_enabled(self.cfg.hotkeys);
            }
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

    /// Re-reads the packs folder every few seconds, so packs you copy in by hand show up.
    fn refresh_packs(&mut self, force: bool) {
        if force || self.packs_read.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            self.packs = packs::list();
            self.packs_read = Some(Instant::now());
        }
    }

    fn live(&self) -> announcer::Live {
        self.sim.state.lock().unwrap().live.clone()
    }

    fn pack_in_use(&self) -> Option<Pack> {
        announcer::pack_in_use(&self.cfg, &self.live())
    }

    fn preview(&self, cue: Cue, recordings: Vec<PathBuf>, pack: Option<&Pack>) {
        let title = crate::announcements::script(cue).title.to_string();
        self.audio.play(Item { title, recordings, chime: pack.is_some_and(|p| p.manifest.chime) });
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

    fn start_fenix_import(&mut self) {
        if self.importing.is_some() {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(packs::import_fenix());
        });
        self.importing = Some(rx);
        self.info("Importing Fenix announcements…");
    }

    fn poll_jobs(&mut self) {
        if let Some(rx) = &self.installing {
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
        if let Some(rx) = &self.importing {
            match rx.try_recv() {
                Ok(Ok(reports)) => {
                    self.importing = None;
                    let names: Vec<&str> = reports.iter().map(|r| r.id.as_str()).collect();
                    let files: usize = reports.iter().map(|r| r.sorted).sum();
                    self.info(format!("Imported {} recordings into {}.", files, names.join(", ")));
                    self.refresh_packs(true);
                }
                Ok(Err(e)) => {
                    self.importing = None;
                    self.error(e);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.importing = None,
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let s = self.sim.state.lock().unwrap().clone();
        let a = self.audio.status.lock().unwrap().clone();
        ui.horizontal(|ui| {
            match &s.connection {
                Connection::Connected => ui.colored_label(GREEN, "Connected to MSFS"),
                Connection::NotRunning => ui.colored_label(Color32::GRAY, "Waiting for MSFS 2024…"),
                Connection::Error(e) => ui.colored_label(RED, e.clone()),
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
                    Some(Model::B737) => ui.label(RichText::new("PMDG 737 data").color(GREEN)),
                    Some(Model::B777) => ui.label(RichText::new("PMDG 777 data").color(GREEN)),
                    None => ui.label(gray("generic sim data")),
                };
                if s.telemetry.gsx.boarding + s.telemetry.gsx.departure + s.telemetry.gsx.deboarding > 0 {
                    ui.label(RichText::new("GSX").color(GREEN));
                }
                ui.separator();
                match &s.live.airline {
                    Some(a) => ui.label(format!("Airline: {} (from the {})", if a.name.is_empty() { &a.icao } else { &a.name }, a.source)),
                    None => ui.label(gray("Airline not recognised")),
                };
            });
        }
        // PMDG aircraft loaded but no SDK data after a while: the broadcast is off.
        let is_pmdg = s.aircraft.to_ascii_uppercase().contains("PMDG");
        if is_pmdg && s.pmdg.is_none() {
            let since = *self.pmdg_seen_at.get_or_insert_with(Instant::now);
            if since.elapsed() > Duration::from_secs(15) {
                ui.horizontal(|ui| {
                    ui.colored_label(AMBER, "No PMDG data yet. The SDK broadcast may be switched off.");
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
            match self.pack_in_use() {
                Some(p) => ui.label(format!("Pack: {}", p.title())),
                None => ui.colored_label(AMBER, "No sound pack in use: announcements will be silent"),
            };
            ui.separator();
            match &a.now_playing {
                Some(t) => ui.label(format!("Now playing: {t}")),
                None => ui.label(gray("Nothing playing")),
            };
            if !a.queued.is_empty() {
                ui.label(gray(format!("then {}", a.queued.join(", "))));
            }
            if let Some(m) = &a.music {
                ui.label(gray(format!("♪ {m}")));
            }
            if (a.now_playing.is_some() || !a.queued.is_empty()) && ui.button("Stop").clicked() {
                self.audio.stop();
            }
            if ui.button(if a.paused { "Resume sound" } else { "Pause sound" }).clicked() {
                self.audio.pause(!a.paused);
            }
        });
        if let Some(e) = &a.error {
            ui.colored_label(RED, e);
        }
        if let Some((m, is_err)) = self.message.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(if is_err { RED } else { BLUE }, m);
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
                match simbrief::fetch(&self.cfg.simbrief_user) {
                    Ok(f) => {
                        let msg = format!("Loaded {}: {} to {}", f.flight_number, f.origin, f.destination);
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
            ui.label("Airline code (ICAO)");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut f.airline_icao).desired_width(60.0).hint_text("auto"));
                let hint = match &s.live.airline {
                    Some(a) if f.airline_icao.is_empty() => format!("detected: {} ({}) from the {}", a.name, a.icao, a.source),
                    _ => "e.g. NAX for Norwegian. Blank detects it from your livery.".to_string(),
                };
                ui.label(gray(hint));
            });
            ui.end_row();
            if !f.flight_number.is_empty() {
                ui.label("Flight");
                ui.label(format!("{}  {} to {}", f.flight_number, f.origin, f.destination));
                ui.end_row();
            }
            ui.label("Scheduled departure");
            ui.horizontal(|ui| {
                match f.scheduled_departure {
                    Some(t) => {
                        let secs = t.rem_euclid(86_400);
                        ui.label(format!("{:02}:{:02}Z", secs / 3600, secs / 60 % 60));
                        if ui.small_button("Clear").clicked() {
                            f.scheduled_departure = None;
                        }
                    }
                    None => {
                        ui.label(gray("none (load a SimBrief plan for the late-departure announcement)"));
                    }
                };
            });
            ui.end_row();
        });

        ui.add_space(12.0);
        ui.heading("What the sim says");
        if s.connection != Connection::Connected {
            ui.label("Start MSFS 2024 and this fills in on its own.");
        } else {
            let t = &s.telemetry;
            let gsx = |v: u8| match v {
                0 => "not running",
                1 => "available",
                2 => "not available",
                3 => "bypassed",
                GSX_REQUESTED => "requested",
                GSX_PERFORMING => "in progress",
                GSX_COMPLETED => "completed",
                _ => "?",
            };
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
                ui.label("GSX boarding");
                ui.label(gsx(t.gsx.boarding));
                ui.label("GSX pushback");
                ui.label(gsx(t.gsx.departure));
                ui.end_row();
                ui.label("Camera");
                ui.label(match s.view {
                    View::Cabin => "cabin",
                    View::Cockpit => "cockpit",
                    View::Outside => "outside",
                });
                ui.label("Time of day");
                ui.label(s.live.daypart().unwrap_or("?"));
                ui.end_row();
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("New flight (back to boarding)").clicked() {
                self.sim.send(Command::NewFlight);
            }
            ui.label(gray("Use this for a turnaround without reloading the flight."));
        });

        ui.add_space(12.0);
        ui.heading("Starting up");
        let mut on = self.autostart;
        if ui.checkbox(&mut on, "Start Cabin Announcer with MSFS 2024, and close it when the sim closes").changed() {
            match autostart::set_enabled(on) {
                Ok(_) => {
                    self.autostart = autostart::is_enabled();
                    self.info(if on { "Added to MSFS's startup list (exe.xml)." } else { "Removed from MSFS's startup list." });
                }
                Err(e) => self.error(format!("Couldn't change exe.xml: {e}")),
            }
        }
        if self.autostart {
            ui.label(gray("MSFS starts this copy of the app. If you move it, untick and tick this again."));
        }
    }

    fn announcements_tab(&mut self, ui: &mut egui::Ui) {
        let played = self.sim.state.lock().unwrap().played.clone();
        self.refresh_packs(false);
        let in_use = self.pack_in_use();
        let pack = match &self.preview_pack {
            Some(id) => self.packs.iter().find(|p| &p.id == id).cloned(),
            None => in_use.clone(),
        };
        ui.label("These play by themselves as the flight goes on. Untick any you don't want.");
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
                ui.colored_label(AMBER, "No packs yet: install or import one on the Sound packs tab.");
            } else if self.preview_pack.is_some() {
                ui.label(gray("Previewing only; flights still use the pack in use now."));
            }
        });
        if let Some(p) = &pack {
            ui.label(gray(format!("{} has {} of {} announcements. The rest stay silent.", p.title(), p.covers.len(), SCRIPTS.len())));
        }
        ui.separator();
        let daypart = self.live().daypart();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for s in SCRIPTS {
                let mut settings = self.cfg.announcement(s.key);
                let before = settings.clone();
                // Following the pack in use includes gap filling, like a flight would.
                let source = if self.preview_pack.is_none() { announcer::source_for(s.key, pack.as_ref(), &self.cfg) } else { pack.clone() };
                let filled_by = source.as_ref().filter(|src| pack.as_ref().is_some_and(|p| p.id != src.id)).map(|src| src.title().to_string());
                let versions = packs::versions(s.key, source.as_ref());
                let pack_title = pack.as_ref().map_or("no pack".to_string(), |p| p.title().to_string());
                let pack = source;
                ui.push_id(s.key, |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut settings.enabled, "");
                        let done = if played.contains(&s.cue) { " ✔" } else { "" };
                        let title = RichText::new(format!("{}{done}", s.title)).strong();
                        ui.label(if versions.is_empty() { title.color(Color32::GRAY) } else { title });
                        ui.label(gray(s.role.label()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !versions.is_empty() && ui.button("▶ Play").on_hover_text("Plays the one a flight would pick right now").clicked() {
                                self.preview(s.cue, packs::recordings(s.key, pack.as_ref(), daypart), pack.as_ref());
                            }
                        });
                    });
                    ui.label(RichText::new(s.when).small().color(Color32::GRAY));
                    if let Some(f) = &filled_by {
                        ui.label(RichText::new(format!("Filled in from {f}")).small().color(BLUE));
                    }
                    match versions.len() {
                        0 => {
                            ui.label(RichText::new(format!("No recording in {pack_title}: stays silent")).small().color(AMBER));
                        }
                        1 => {
                            ui.label(RichText::new(format!("Recording: {}", file_name(&versions[0]))).small().color(BLUE));
                        }
                        n => {
                            ui.collapsing(format!("{n} recorded versions"), |ui| {
                                for f in &versions {
                                    ui.horizontal(|ui| {
                                        if ui.small_button("▶").clicked() {
                                            self.preview(s.cue, vec![f.clone()], pack.as_ref());
                                        }
                                        ui.label(file_name(f));
                                    });
                                }
                            });
                        }
                    }
                });
                if settings != before {
                    self.cfg.announcements.insert(s.key.to_string(), settings);
                }
                ui.separator();
            }
            // Background layers, previewable here too.
            for (folder, title) in [(BOARDING_MUSIC, "Boarding music"), (CABIN_AMBIENCE, "Cabin ambience")] {
                let tracks = packs::background(folder, pack.as_ref());
                ui.push_id(folder, |ui| {
                    ui.label(RichText::new(title).strong());
                    if tracks.is_empty() {
                        ui.label(RichText::new("None in this pack").small().color(Color32::GRAY));
                    }
                    for f in &tracks {
                        ui.horizontal(|ui| {
                            if ui.small_button("▶").clicked() {
                                self.audio.play(Item { title: format!("{title}: {}", file_name(f)), recordings: vec![f.clone()], chime: false });
                            }
                            ui.label(file_name(f));
                        });
                    }
                });
                ui.separator();
            }
        });
    }

    fn packs_tab(&mut self, ui: &mut egui::Ui) {
        self.refresh_packs(false);
        let detected = self.live().airline;
        let in_use = self.pack_in_use();

        ui.label(
            "A sound pack is a set of real recorded announcements. The pack for your airline plays automatically; \
             a pack without an airline plays for every airline that doesn't have its own.",
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Use");
            let shown = match self.cfg.sound.pack.as_str() {
                "" => "Automatic (match the airline)".to_string(),
                "none" => "No pack (silent)".to_string(),
                id => self.packs.iter().find(|p| p.id == id).map_or(id.to_string(), |p| p.title().to_string()),
            };
            egui::ComboBox::from_id_salt("pack").selected_text(shown).width(260.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.cfg.sound.pack, String::new(), "Automatic (match the airline)");
                ui.selectable_value(&mut self.cfg.sound.pack, "none".to_string(), "No pack (silent)");
                for p in &self.packs {
                    ui.selectable_value(&mut self.cfg.sound.pack, p.id.clone(), p.title());
                }
            });
            let now = match (&in_use, &detected) {
                (Some(p), _) => format!("in use now: {}", p.title()),
                (None, Some(a)) => format!("no pack for {} yet", if a.name.is_empty() { &a.icao } else { &a.name }),
                (None, None) => "airline not recognised yet".to_string(),
            };
            ui.label(gray(now));
        });

        ui.add_space(10.0);
        ui.heading("Fenix A320 announcements");
        ui.horizontal(|ui| {
            if self.fenix_sets == 0 {
                ui.label(gray("None found in your Community folder."));
            } else {
                ui.label(format!("Found {} Fenix-format announcement set(s) in your Community folder.", self.fenix_sets));
            }
            let busy = self.importing.is_some();
            if ui.add_enabled(!busy, egui::Button::new(if busy { "Importing…" } else { "Import them" })).clicked() {
                self.start_fenix_import();
            }
            if ui.small_button("Look again").clicked() {
                self.fenix_sets = packs::find_fenix_sets().len();
            }
        });
        ui.label(gray(
            "Each airline folder (BAW, EZY, …) becomes its own pack for that airline, and \"Default\" plays for the rest. \
             Importing again refreshes them. Universal Announcer packs use the same format.",
        ));

        ui.add_space(10.0);
        ui.heading("Install a pack");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.install_source).hint_text("zip file, folder, or https:// download link").desired_width(380.0));
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
             (Fenix names, or words like \"safety\", \"boarding\", \"turbulence\"). Leave the airline blank for a pack that plays on every airline.",
        ));
        ui.label(gray("Only install recordings you have the right to use. Real airline announcements are copyrighted."));

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
                        let mark = if in_use.as_ref().is_some_and(|a| a.id == p.id) { "▶ " } else { "" };
                        ui.label(RichText::new(format!("{mark}{}", p.title())).strong());
                        ui.label(gray(format!("{} of {} announcements", p.covers.len(), SCRIPTS.len())));
                        if p.music > 0 {
                            ui.label(gray(format!("♪ {} music", p.music)));
                        }
                        if p.ambience > 0 {
                            ui.label(gray(format!("{} ambience", p.ambience)));
                        }
                        if p.unsorted > 0 {
                            ui.colored_label(AMBER, format!("{} unsorted", p.unsorted));
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
                        let r = ui.add(egui::TextEdit::singleline(&mut codes).hint_text("all").desired_width(120.0));
                        if r.lost_focus() && codes != m.airlines.join(", ") {
                            m.airlines = codes.split([',', ' ']).filter(|c| !c.is_empty()).map(|c| c.to_ascii_uppercase()).collect();
                            changed = Some((p.id.clone(), m.clone()));
                        }
                        if ui.checkbox(&mut m.play_all, "Play every file in order").on_hover_text("For packs with the same announcement in two languages").changed() {
                            changed = Some((p.id.clone(), m.clone()));
                        }
                        if ui.checkbox(&mut m.chime, "Add a PA chime").on_hover_text("For recordings that don't start with their own chime").changed() {
                            changed = Some((p.id.clone(), m));
                        }
                    });
                    let missing: Vec<&str> = SCRIPTS.iter().filter(|s| !p.covers.contains(&s.key)).map(|s| s.title).collect();
                    if !missing.is_empty() {
                        ui.label(RichText::new(format!("Not in this pack: {}", missing.join(", "))).small().color(Color32::GRAY));
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
                    if self.cfg.sound.pack == id {
                        self.cfg.sound.pack.clear();
                    }
                    self.info(format!("Removed {id}"));
                    self.refresh_packs(true);
                }
                Err(e) => self.error(e),
            }
        }
    }

    fn sound_tab(&mut self, ui: &mut egui::Ui) {
        let view = self.sim.state.lock().unwrap().view;
        let v = &mut self.cfg.sound;
        egui::Grid::new("sound").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Announcements");
            ui.add(egui::Slider::new(&mut v.volume, 0.0..=1.0));
            ui.end_row();
            ui.label("Boarding music");
            ui.horizontal(|ui| {
                ui.checkbox(&mut v.music, "");
                ui.add_enabled(v.music, egui::Slider::new(&mut v.music_volume, 0.0..=1.0));
                ui.label(gray("at the gate, before pushback and while deboarding"));
            });
            ui.end_row();
            ui.label("Cabin ambience");
            ui.horizontal(|ui| {
                ui.checkbox(&mut v.ambience, "");
                ui.add_enabled(v.ambience, egui::Slider::new(&mut v.ambience_volume, 0.0..=1.0));
                ui.label(gray("from doors closed to the gate, if the pack has it"));
            });
            ui.end_row();
            ui.label("Cabin PA sound");
            ui.checkbox(&mut v.pa_effect, "Make clean studio recordings sound like the cabin speakers");
            ui.end_row();
            ui.label("Missing announcements");
            ui.checkbox(&mut v.fill_gaps, "Fill gaps in an airline's pack from a pack without an airline (e.g. Fenix Default)");
            ui.end_row();
        });

        ui.add_space(12.0);
        ui.heading("Where you're sitting");
        ui.label(gray(format!(
            "Now: {}. The PA sounds different depending on the camera you're using.",
            match view {
                View::Cabin => "cabin",
                View::Cockpit => "cockpit",
                View::Outside => "outside",
            }
        )));
        egui::Grid::new("views").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Cockpit camera");
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut v.cockpit_volume, 0.0..=1.0));
                ui.checkbox(&mut v.cockpit_muffle, "Muffled, as if through the flight deck door");
            });
            ui.end_row();
            ui.label("Outside cameras");
            ui.add(egui::Slider::new(&mut v.outside_volume, 0.0..=1.0));
            ui.end_row();
        });

        ui.add_space(12.0);
        ui.heading("Hotkeys");
        ui.checkbox(&mut self.cfg.hotkeys, "Use hotkeys (they work while the sim has focus, and in VR)");
        if self.hotkeys_failed.load(Ordering::SeqCst) && self.cfg.hotkeys {
            ui.colored_label(AMBER, "Another program is already using one of these keys.");
        }
        egui::Grid::new("hotkeys").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
            for (keys, what) in hotkeys::HELP {
                ui.label(RichText::new(*keys).monospace());
                ui.label(*what);
                ui.end_row();
            }
        });
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.sim.state.lock().unwrap().quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        egui::TopBottomPanel::top("status").show(ctx, |ui| {
            ui.add_space(4.0);
            self.status_bar(ui);
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (Tab::Flight, "Flight"),
                    (Tab::Announcements, "Announcements"),
                    (Tab::Packs, "Sound packs"),
                    (Tab::Sound, "Sound"),
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
            Tab::Sound => self.sound_tab(ui),
            Tab::Log => self.log_tab(ui),
        });
        // Dropping a zip or folder anywhere on the window installs it.
        let dropped: Vec<String> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.as_ref().map(|p| p.display().to_string())).collect());
        if let Some(first) = dropped.into_iter().next() {
            self.tab = Tab::Packs;
            self.start_install(first);
        }
        self.poll_jobs();
        self.sync_config();
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.cfg.save();
    }
}
