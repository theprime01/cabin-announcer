//! Settings, saved as TOML in %APPDATA%\CabinAnnouncer\config.toml.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Flight {
    /// Spoken name; empty uses the detected airline.
    pub airline: String,
    /// ICAO code from the flight plan, e.g. "NAX". Empty detects it from the livery.
    pub airline_icao: String,
    /// As filed, e.g. "BAW117"; spoken as its digits.
    pub flight_number: String,
    pub origin: String,
    pub destination: String,
    pub captain: String,
    /// Block time, used for "our flight time today".
    pub flight_time_minutes: Option<u32>,
    pub cruise_altitude_ft: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Voice {
    /// Windows voice names; empty picks one automatically.
    pub captain_voice: String,
    pub purser_voice: String,
    /// 1.0 is normal speed.
    pub rate: f64,
    /// 0..=1.
    pub volume: f32,
    /// Band-limits speech so it sounds like the cabin PA.
    pub pa_effect: bool,
    /// Apply the PA effect to your own recordings too.
    pub pa_effect_on_recordings: bool,
    pub chime: bool,
    /// Sound pack: empty picks one for the detected airline, "none" uses speech only.
    pub pack: String,
}

impl Default for Voice {
    fn default() -> Self {
        Voice {
            captain_voice: String::new(),
            purser_voice: String::new(),
            rate: 0.95,
            volume: 0.8,
            pa_effect: true,
            pa_effect_on_recordings: false,
            chime: true,
            pack: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AnnouncementSettings {
    pub enabled: bool,
    /// Replaces the built-in script when set.
    pub text: Option<String>,
}

impl Default for AnnouncementSettings {
    fn default() -> Self {
        AnnouncementSettings { enabled: true, text: None }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub simbrief_user: String,
    pub flight: Flight,
    pub voice: Voice,
    /// Keyed by script key, e.g. "safety_demo".
    pub announcements: BTreeMap<String, AnnouncementSettings>,
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("CabinAnnouncer")
}

/// Your own recordings: sounds\<script key>\*.wav|mp3|ogg|flac. One is picked at random.
pub fn sounds_dir() -> PathBuf {
    data_dir().join("sounds")
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
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
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
    fn round_trips_and_tolerates_missing_fields() {
        let mut c = Config::default();
        c.flight.airline = "Speedbird".into();
        c.announcements.insert("safety_demo".into(), AnnouncementSettings { enabled: false, text: None });
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        let partial: Config = toml::from_str("[voice]\nvolume = 0.5\n").unwrap();
        assert_eq!(partial.voice.volume, 0.5);
        assert!(partial.voice.chime);
    }
}
