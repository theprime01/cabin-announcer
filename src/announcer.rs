//! Turns a cue into recordings for the audio thread, from the pack in use.

use crate::airline::Detected;
use crate::announcements::{daypart, script};
use crate::audio::Item;
use crate::config::Config;
use crate::packs::{self, Pack};
use crate::phase::Cue;

/// Values that come from the sim rather than the settings.
#[derive(Clone, Debug, Default)]
pub struct Live {
    /// Seconds since local midnight at the aircraft.
    pub local_time: Option<f64>,
    /// The airline worked out from the flight plan or livery.
    pub airline: Option<Detected>,
}

impl Live {
    pub fn daypart(&self) -> Option<&'static str> {
        self.local_time.map(daypart)
    }
}

pub fn pack_in_use(config: &Config, live: &Live) -> Option<Pack> {
    packs::active(&config.sound.pack, live.airline.as_ref().map(|a| a.icao.as_str()))
}

/// The pack that will actually play this announcement (or music folder):
/// the pack in use, or, when it hasn't got one and gap filling is on, a pack
/// that isn't tied to an airline (e.g. "Fenix Default").
pub fn source_for(key: &str, pack: Option<&Pack>, config: &Config) -> Option<Pack> {
    let pack = pack?;
    if !packs::versions(key, Some(pack)).is_empty() {
        return Some(pack.clone());
    }
    if !config.sound.fill_gaps {
        return None;
    }
    packs::list()
        .into_iter()
        .find(|p| p.id != pack.id && p.manifest.airlines.is_empty() && !packs::versions(key, Some(p)).is_empty())
}

pub enum Outcome {
    Play(Item),
    SwitchedOff,
    NoRecording,
}

/// What to play for a cue; `force` ignores the on/off switch (for previews).
pub fn item(cue: Cue, config: &Config, live: &Live, pack: Option<&Pack>, force: bool) -> Outcome {
    let s = script(cue);
    if !force && !config.announcement(s.key).enabled {
        return Outcome::SwitchedOff;
    }
    let source = source_for(s.key, pack, config);
    let recordings = packs::recordings(s.key, source.as_ref(), live.daypart());
    if recordings.is_empty() {
        return Outcome::NoRecording;
    }
    Outcome::Play(Item { title: s.title.to_string(), recordings, chime: source.is_some_and(|p| p.manifest.chime) })
}
