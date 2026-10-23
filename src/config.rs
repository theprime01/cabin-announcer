//! Settings, saved as TOML in %APPDATA%\CabinAnnouncer\config.toml.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Flight {
    /// ICAO code, e.g. "NAX". Empty detects it from the livery.
    pub airline_icao: String,
    /// From the SimBrief plan, for display.
    pub flight_number: String,
    pub origin: String,
    pub destination: String,
    /// Scheduled off-block time from SimBrief, as a Unix timestamp (UTC).
    pub scheduled_departure: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Sound {
    /// Sound pack: empty picks one for the detected airline, "none" is silent.
    pub pack: String,
    /// When the airline's pack lacks an announcement, use one from a pack
    /// that isn't tied to an airline (such as Fenix Default).
    pub fill_gaps: bool,
    /// 0..=1 for announcements.
    pub volume: f32,
    pub music: bool,
    pub music_volume: f32,
    pub ambience: bool,
    pub ambience_volume: f32,
    /// Band-limits recordings so clean studio audio sounds like the cabin PA.
    pub pa_effect: bool,
    /// Volume (0..=1) and muffling with the cockpit camera: the PA heard
    /// through the flight deck door.
    pub cockpit_volume: f32,
    pub cockpit_muffle: bool,
    /// Volume with outside cameras (drone, chase, tower).
    pub outside_volume: f32,
}

impl Default for Sound {
    fn default() -> Self {
        Sound {
            pack: String::new(),
            fill_gaps: true,
            volume: 0.8,
            music: true,
            music_volume: 0.35,
            ambience: false,
            ambience_volume: 0.3,
            pa_effect: false,
            cockpit_volume: 0.6,
            cockpit_muffle: true,
            outside_volume: 0.3,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AnnouncementSettings {
    pub enabled: bool,
}

impl Default for AnnouncementSettings {
    fn default() -> Self {
        AnnouncementSettings { enabled: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub simbrief_user: String,
    pub flight: Flight,
    pub sound: Sound,
    /// Ctrl+Shift+F9 replay, F10 stop, F11 pause.
    pub hotkeys: bool,
    /// Keyed by script key, e.g. "safety_demo".
    pub announcements: BTreeMap<String, AnnouncementSettings>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            simbrief_user: String::new(),
            flight: Flight::default(),
            sound: Sound::default(),
            hotkeys: true,
            announcements: BTreeMap::new(),
        }
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("CabinAnnouncer")
}

/// Installed sound packs: packs\<pack>\<script key>\*.ogg, plus an optional pack.toml.
pub fn packs_dir() -> PathBuf {
    data_dir().join("packs")
}

fn path() -> PathBuf {
    data_dir().join("config.toml")
}

impl Config {
    pub fn load() -> Config {
        std::fs::read_to_string(path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
        std::fs::write(path(), text).map_err(|e| e.to_string())
    }

    pub fn announcement(&self, key: &str) -> AnnouncementSettings {
        self.announcements.get(key).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_old_files() {
        let mut c = Config::default();
        c.flight.airline_icao = "NAX".into();
        c.announcements.insert("safety_demo".into(), AnnouncementSettings { enabled: false });
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        // A settings file from the speech version still loads.
        let old = "[voice]\nvolume = 0.5\n[flight]\nairline = \"Speedbird\"\ncaptain = \"Smith\"\n[announcements.safety_demo]\nenabled = false\ntext = \"hi\"\n";
        let loaded: Config = toml::from_str(old).unwrap();
        assert!(!loaded.announcement("safety_demo").enabled);
        assert!(loaded.hotkeys);
    }
}
