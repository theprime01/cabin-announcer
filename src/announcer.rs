//! Turns a cue into something the audio thread can play, using the settings
//! and live values from the sim.

use crate::announcements::{self, Values, script};
use crate::audio::Item;
use crate::config::Config;
use crate::airline::Detected;
use crate::packs;
use crate::phase::Cue;

/// Values that come from the sim rather than the settings.
#[derive(Clone, Debug, Default)]
pub struct Live {
    /// Seconds since local midnight at the aircraft.
    pub local_time: Option<f64>,
    pub temperature_c: Option<f64>,
    /// Minutes to the destination, when it can be worked out.
    pub minutes_remaining: Option<f64>,
    /// Flight number typed into the FMC.
    pub fmc_flight_number: String,
    /// Cruise altitude typed into the FMC (feet).
    pub fmc_cruise_alt: Option<f64>,
    /// Flight number the livery or sim's ATC uses.
    pub atc_flight_number: String,
    /// The airline worked out from the flight plan or livery.
    pub airline: Option<Detected>,
}

pub fn values(config: &Config, live: &Live) -> Values {
    let f = &config.flight;
    let mut v = Values::new();
    let number = [&f.flight_number, &live.fmc_flight_number, &live.atc_flight_number]
        .into_iter()
        .find(|n| !n.trim().is_empty())
        .cloned()
        .unwrap_or_default();
    let detected = live.airline.as_ref().map(|a| a.name.clone()).unwrap_or_default();
    v.insert("airline", if f.airline.trim().is_empty() { detected } else { f.airline.clone() });
    v.insert("flight", if number.trim().is_empty() { String::new() } else { announcements::spoken_flight_number(&number) });
    v.insert("origin", f.origin.clone());
    v.insert("destination", f.destination.clone());
    v.insert("captain", f.captain.clone());
    v.insert("daypart", live.local_time.map_or("day", announcements::daypart).to_string());
    if let Some(m) = f.flight_time_minutes.filter(|&m| m > 0) {
        v.insert("flight_time", announcements::spoken_duration(m as f64));
    }
    let cruise = f.cruise_altitude_ft.map(|a| a as f64).or(live.fmc_cruise_alt).filter(|&a| a > 0.0);
    if let Some(a) = cruise {
        v.insert("cruise_altitude", announcements::spoken_altitude(a));
    }
    if let Some(m) = live.minutes_remaining.filter(|&m| m >= 1.0) {
        v.insert("time_remaining", announcements::spoken_duration(m));
    }
    if let Some(t) = live.local_time {
        v.insert("local_time", announcements::spoken_time(t));
    }
    if let Some(c) = live.temperature_c {
        v.insert("temperature", announcements::spoken_temperature(c));
    }
    v
}

pub fn text_for(cue: Cue, config: &Config, live: &Live) -> String {
    let s = script(cue);
    let custom = config.announcement(s.key).text.filter(|t| !t.trim().is_empty());
    announcements::render(custom.as_deref().unwrap_or(s.text), &values(config, live))
}

/// None when the announcement is switched off (unless `force`, for test plays).
pub fn item(cue: Cue, config: &Config, live: &Live, force: bool) -> Option<Item> {
    let s = script(cue);
    if !force && !config.announcement(s.key).enabled {
        return None;
    }
    let pack = packs::active(&config.voice.pack, live.airline.as_ref().map(|a| a.icao.as_str()));
    Some(Item {
        title: s.title.to_string(),
        role: s.role,
        text: text_for(cue, config, live),
        chime: s.chime,
        recordings: packs::recordings(s.key, pack.as_ref()),
        voice: config.voice.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_from_settings_and_sim() {
        let mut c = Config::default();
        c.flight.airline = "Speedbird".into();
        c.flight.destination = "New York".into();
        let live = Live {
            local_time: Some(15.0 * 3600.0),
            temperature_c: Some(21.0),
            fmc_flight_number: "BAW117".into(),
            ..Default::default()
        };
        let t = text_for(Cue::BoardingWelcome, &c, &live);
        assert!(t.starts_with("Good afternoon, ladies and gentlemen, and welcome aboard Speedbird flight 1 1 7 to New York."), "{t}");
        let t = text_for(Cue::AfterLanding, &c, &live);
        assert!(t.contains("welcome to New York. The local time is 3:00 PM. The temperature outside is 21 degrees Celsius"), "{t}");
    }
}
