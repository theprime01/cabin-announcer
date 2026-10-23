#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod airline;
mod announcements;
mod announcer;
mod audio;
mod config;
mod packs;
mod phase;
mod pmdg;
mod sim;
mod simbrief;
mod simconnect;
mod ui;

use std::sync::{Arc, Mutex};

fn main() -> eframe::Result<()> {
    let config = Arc::new(Mutex::new(config::Config::load()));
    let audio = audio::Audio::start();
    let sim = sim::Sim::start(config.clone(), audio.clone());
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Cabin Announcer")
            .with_inner_size([860.0, 680.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Cabin Announcer",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, config, audio, sim)))),
    )
}
