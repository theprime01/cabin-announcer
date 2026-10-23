//! The announcement catalog: who says what for each cue, and the templating
//! that fills in flight details.
//!
//! Placeholders look like `{destination}`. A `[bracketed part]` is dropped
//! when any placeholder inside it has no value, so scripts read naturally
//! whether or not a flight plan is loaded.

use std::collections::HashMap;

use crate::phase::Cue;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Role {
    Captain,
    Purser,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Captain => "Captain",
            Role::Purser => "Cabin crew",
        }
    }
}

pub struct Script {
    pub cue: Cue,
    /// Stable id used in the config file and for the custom sounds folder.
    pub key: &'static str,
    pub title: &'static str,
    pub role: Role,
    /// Played with a PA chime first.
    pub chime: bool,
    pub text: &'static str,
}

pub const SCRIPTS: &[Script] = &[
    Script {
        cue: Cue::BoardingWelcome,
        key: "boarding_welcome",
        title: "Boarding welcome",
        role: Role::Purser,
        chime: true,
        text: "Good {daypart}, ladies and gentlemen, and welcome aboard[ {airline}][ flight {flight}][ to {destination}]. \
               As you find your seat, please place your bags in the overhead lockers or under the seat in front of you, \
               and step out of the aisle so that others can get by. Thank you.",
    },
    Script {
        cue: Cue::ArmDoors,
        key: "arm_doors",
        title: "Arm doors",
        role: Role::Purser,
        chime: false,
        text: "Cabin crew, please arm the doors and cross-check.",
    },
    Script {
        cue: Cue::CaptainWelcome,
        key: "captain_welcome",
        title: "Captain's welcome",
        role: Role::Captain,
        chime: true,
        text: "Ladies and gentlemen, good {daypart} from the flight deck. This is your captain[, {captain},] speaking. \
               On behalf of the whole crew, welcome aboard[ this {airline} flight][ to {destination}]. \
               [Our flight time today is expected to be {flight_time}.] [We'll be cruising at {cruise_altitude}.] \
               We'll be on our way shortly. In the meantime, please give your full attention to the cabin crew \
               for the safety demonstration. Thank you.",
    },
    Script {
        cue: Cue::SafetyDemo,
        key: "safety_demo",
        title: "Safety demonstration",
        role: Role::Purser,
        chime: true,
        text: "Ladies and gentlemen, please give us your attention for the safety demonstration, even if you fly with us often. \
               To fasten your seat belt, insert the metal tip into the buckle and pull the strap tight. To release it, lift the buckle. \
               Keep your seat belt fastened whenever you are seated. \
               This aircraft has several emergency exits. Please take a moment now to find the one nearest to you, \
               remembering that it may be behind you. In an emergency, lighting in the floor will guide you to an exit. \
               If the cabin loses pressure, oxygen masks will drop from the panel above you. Pull a mask towards you, \
               place it over your nose and mouth, and breathe normally. Fit your own mask before helping others. \
               Your life jacket is under your seat. Put it over your head, fasten the straps around your waist, \
               and only inflate it once you have left the aircraft. \
               Smoking, including electronic cigarettes, is not allowed anywhere on board. \
               Please make sure your seat back is upright, your tray table is stowed, and your window blind is open for takeoff. \
               The safety card in your seat pocket has more information. Thank you for your attention.",
    },
    Script {
        cue: Cue::CrewSeatsTakeoff,
        key: "crew_seats_takeoff",
        title: "Cabin crew, seats for takeoff",
        role: Role::Captain,
        chime: false,
        text: "Cabin crew, please take your seats for takeoff.",
    },
    Script {
        cue: Cue::BeltsOffClimb,
        key: "belts_off_climb",
        title: "Seat belt sign off (climb)",
        role: Role::Purser,
        chime: true,
        text: "Ladies and gentlemen, the captain has switched off the fasten seat belt sign. You are now free to move around the cabin, \
               but whenever you are seated, please keep your seat belt loosely fastened. \
               The cabin crew will shortly be coming through the cabin with our onboard service.",
    },
    Script {
        cue: Cue::CaptainCruise,
        key: "captain_cruise",
        title: "Captain's cruise update",
        role: Role::Captain,
        chime: true,
        text: "Ladies and gentlemen, this is your captain again. [We've now reached our cruising altitude of {cruise_altitude}.] \
               [We have around {time_remaining} to go to {destination}.] \
               Please sit back and enjoy the flight, and I'll speak to you again before we start our descent.",
    },
    Script {
        cue: Cue::Turbulence,
        key: "turbulence",
        title: "Turbulence, seat belts on",
        role: Role::Purser,
        chime: true,
        text: "Ladies and gentlemen, the captain has switched on the fasten seat belt sign. \
               Please return to your seats and fasten your seat belts. The toilets should not be used while the sign is on. Thank you.",
    },
    Script {
        cue: Cue::CaptainDescent,
        key: "captain_descent",
        title: "Captain's descent announcement",
        role: Role::Captain,
        chime: true,
        text: "Ladies and gentlemen, from the flight deck. We'll shortly begin our descent[ into {destination}]. \
               [We expect to be on the ground in about {time_remaining}.] \
               The cabin crew will soon be coming through the cabin to prepare for landing. Thank you.",
    },
    Script {
        cue: Cue::PrepareCabinLanding,
        key: "prepare_cabin_landing",
        title: "Prepare the cabin for landing",
        role: Role::Purser,
        chime: true,
        text: "Ladies and gentlemen, we have started our approach[ into {destination}]. Please return to your seats and fasten your seat belts. \
               Make sure your seat back is upright, your tray table is stowed, and your window blind is open. \
               Please stow any large electronic devices, and make sure your bags are under the seat in front of you or in the overhead lockers. \
               Thank you.",
    },
    Script {
        cue: Cue::CrewSeatsLanding,
        key: "crew_seats_landing",
        title: "Cabin crew, seats for landing",
        role: Role::Captain,
        chime: false,
        text: "Cabin crew, please take your seats for landing.",
    },
    Script {
        cue: Cue::GoAround,
        key: "go_around",
        title: "Go-around",
        role: Role::Captain,
        chime: true,
        text: "Ladies and gentlemen, this is the captain. As you may have noticed, we've discontinued our approach and are climbing away. \
               This is a normal procedure, and we'll be setting up for another approach shortly. \
               Please stay in your seats with your seat belts fastened. Thank you.",
    },
    Script {
        cue: Cue::AfterLanding,
        key: "after_landing",
        title: "Welcome after landing",
        role: Role::Purser,
        chime: true,
        text: "Ladies and gentlemen, welcome[ to {destination}]. [The local time is {local_time}.] [The temperature outside is {temperature}.] \
               Please stay in your seat with your seat belt fastened until the captain has switched off the fasten seat belt sign. \
               Please don't open the overhead lockers until we have come to a complete stop. \
               On behalf of[ {airline} and] the whole crew, thank you for flying with us today.",
    },
    Script {
        cue: Cue::DisarmDoors,
        key: "disarm_doors",
        title: "Disarm doors",
        role: Role::Purser,
        chime: false,
        text: "Cabin crew, please disarm the doors and cross-check.",
    },
    Script {
        cue: Cue::Disembark,
        key: "disembark",
        title: "Disembarking",
        role: Role::Purser,
        chime: true,
        text: "Ladies and gentlemen, the captain has switched off the fasten seat belt sign. \
               Please take care when opening the overhead lockers, as items may have moved during the flight, \
               and remember to take all of your belongings with you. Thank you for flying[ with {airline}], \
               and we hope to see you again soon. Goodbye.",
    },
];

pub fn script(cue: Cue) -> &'static Script {
    SCRIPTS.iter().find(|s| s.cue == cue).expect("every cue has a script")
}

/// Values for the placeholders; an empty or missing value counts as unknown.
pub type Values = HashMap<&'static str, String>;

fn fill(text: &str, values: &Values) -> (String, bool) {
    let mut out = String::new();
    let mut complete = true;
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            out.push_str(&rest[start..]);
            return (out, complete);
        };
        let name = &rest[start + 1..start + end];
        match values.get(name).filter(|v| !v.trim().is_empty()) {
            Some(v) => out.push_str(v.trim()),
            None => complete = false,
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    (out, complete)
}

/// Fills a script. Bracketed parts with an unknown placeholder are dropped;
/// an unknown placeholder outside brackets is simply left out.
pub fn render(text: &str, values: &Values) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&fill(&rest[..open], values).0);
        let Some(close) = rest[open..].find(']') else {
            rest = &rest[open..];
            break;
        };
        let (part, complete) = fill(&rest[open + 1..open + close], values);
        if complete {
            out.push_str(&part);
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(&fill(rest, values).0);
    // Tidy up spaces left behind by dropped parts.
    let mut tidy = out.split_whitespace().collect::<Vec<_>>().join(" ");
    for (from, to) in [(" .", "."), (" ,", ","), (",.", ".")] {
        tidy = tidy.replace(from, to);
    }
    tidy
}

/// "BAW117" -> "1 1 7", so speech reads it the way crews say it.
pub fn spoken_flight_number(s: &str) -> String {
    let digits: String = s.chars().skip_while(|c| !c.is_ascii_digit()).collect();
    let digits = if digits.is_empty() { s.to_string() } else { digits };
    digits.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
}

pub fn spoken_duration(minutes: f64) -> String {
    let m = minutes.round().max(1.0) as u64;
    let (h, m) = (m / 60, m % 60);
    let plural = |n: u64, w: &str| if n == 1 { format!("1 {w}") } else { format!("{n} {w}s") };
    match (h, m) {
        (0, m) => plural(m, "minute"),
        (h, 0) => plural(h, "hour"),
        (h, m) => format!("{} and {}", plural(h, "hour"), plural(m, "minute")),
    }
}

pub fn spoken_altitude(feet: f64) -> String {
    let rounded = ((feet / 1000.0).round() * 1000.0) as i64;
    format!("{} feet", rounded)
}

/// Seconds since local midnight -> "4:35 PM".
pub fn spoken_time(secs: f64) -> String {
    let mins = (secs / 60.0) as i64 % (24 * 60);
    let (h, m) = (mins / 60, mins % 60);
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12}:{m:02} {ampm}")
}

pub fn daypart(secs: f64) -> &'static str {
    match (secs / 3600.0) as i64 {
        4..=11 => "morning",
        12..=17 => "afternoon",
        _ => "evening",
    }
}

pub fn spoken_temperature(c: f64) -> String {
    let f = c * 9.0 / 5.0 + 32.0;
    format!("{} degrees Celsius, or {} degrees Fahrenheit", c.round() as i64, f.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&'static str, &str)]) -> Values {
        pairs.iter().map(|&(k, v)| (k, v.to_string())).collect()
    }

    #[test]
    fn brackets_drop_unknown_parts() {
        let v = values(&[("airline", "Speedbird"), ("destination", "")]);
        assert_eq!(render("Welcome aboard[ {airline}][ to {destination}].", &v), "Welcome aboard Speedbird.");
        assert_eq!(render("[Time is {local_time}.] Thanks.", &v), "Thanks.");
    }

    #[test]
    fn every_script_renders_without_values() {
        for s in SCRIPTS {
            let text = render(s.text, &Values::new());
            assert!(!text.contains('{') && !text.contains('['), "{}: {text}", s.key);
        }
    }

    #[test]
    fn spoken_forms() {
        assert_eq!(spoken_flight_number("BAW117"), "1 1 7");
        assert_eq!(spoken_duration(135.0), "2 hours and 15 minutes");
        assert_eq!(spoken_duration(60.0), "1 hour");
        assert_eq!(spoken_time(16.0 * 3600.0 + 35.0 * 60.0), "4:35 PM");
        assert_eq!(spoken_time(5.0 * 60.0), "12:05 AM");
        assert_eq!(spoken_altitude(34990.0), "35000 feet");
    }
}
