#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod airline;
mod announcements;
mod announcer;
mod audio;
mod autostart;
mod config;
mod hotkeys;
mod msfs;
mod packs;
mod phase;
mod pmdg;
mod sim;
mod simbrief;
mod simconnect;
mod ui;

use std::sync::{Arc, Mutex};

fn main() -> eframe::Result<()> {
    let autostarted = std::env::args().any(|a| a == autostart::FLAG);
    let config = config::Config::load();
    let audio = audio::Audio::start(config.sound.clone());
    let config = Arc::new(Mutex::new(config));
    let sim = sim::Sim::start(config.clone(), audio.clone(), autostarted);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Cabin Announcer")
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Cabin Announcer",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, config, audio, sim, autostarted)))),
    )
}
