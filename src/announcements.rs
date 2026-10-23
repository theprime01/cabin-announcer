//! The announcement catalog: every automatic announcement, who makes it and
//! when it plays. The audio itself always comes from recordings in a pack.

use crate::phase::Cue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Captain,
    Crew,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Captain => "Flight deck",
            Role::Crew => "Cabin crew",
        }
    }
}

pub struct Script {
    pub cue: Cue,
    /// Folder name inside a pack, and the id used in the settings file.
    pub key: &'static str,
    pub title: &'static str,
    pub role: Role,
    /// When it plays, shown on the Announcements tab.
    pub when: &'static str,
}

/// Folders in a pack that aren't announcements.
pub const BOARDING_MUSIC: &str = "boarding_music";
pub const CABIN_AMBIENCE: &str = "cabin_ambience";

pub const SCRIPTS: &[Script] = &[
    Script { cue: Cue::BoardingWelcome, key: "boarding_welcome", title: "Boarding welcome", role: Role::Crew, when: "Boarding starts (GSX, or a door open at the gate)" },
    Script { cue: Cue::BoardingComplete, key: "boarding_complete", title: "Boarding complete", role: Role::Crew, when: "Boarding finishes or the doors close" },
    Script { cue: Cue::ArmDoors, key: "arm_doors", title: "Arm doors", role: Role::Crew, when: "Doors closed, before pushback" },
    Script { cue: Cue::DepartureDelay, key: "departure_delay", title: "Departure delay", role: Role::Captain, when: "10 min past the SimBrief departure time, or 10 min at the gate with the doors closed" },
    Script { cue: Cue::CaptainWelcome, key: "captain_welcome", title: "Captain's welcome", role: Role::Captain, when: "20 s after the doors close, or at pushback" },
    Script { cue: Cue::PreSafety, key: "pre_safety_briefing", title: "Before the safety briefing", role: Role::Crew, when: "Pushback or start of taxi, just before the safety briefing" },
    Script { cue: Cue::SafetyDemo, key: "safety_demo", title: "Safety briefing", role: Role::Crew, when: "Pushback or start of taxi" },
    Script { cue: Cue::CabinDimTakeoff, key: "cabin_dim_takeoff", title: "Cabin lights dimmed for takeoff", role: Role::Crew, when: "Taxiing at night" },
    Script { cue: Cue::CabinSecureTakeoff, key: "cabin_secure_takeoff", title: "Cabin crew, secure the cabin for takeoff", role: Role::Captain, when: "45 s into the taxi, or entering the runway" },
    Script { cue: Cue::TakeoffDelay, key: "takeoff_delay", title: "Takeoff delay", role: Role::Captain, when: "Stopped for 4 min while taxiing out" },
    Script { cue: Cue::CrewSeatsTakeoff, key: "crew_seats_takeoff", title: "Cabin crew, seats for takeoff", role: Role::Captain, when: "Strobes or landing lights switched on while taxiing (or at 40 kt)" },
    Script { cue: Cue::BeltsOffClimb, key: "belts_off_climb", title: "After takeoff, seat belt sign off", role: Role::Crew, when: "Seat belt sign switched off in the climb" },
    Script { cue: Cue::CaptainCruise, key: "captain_cruise", title: "Captain's cruise update", role: Role::Captain, when: "60 s after levelling off at cruise" },
    Script { cue: Cue::Turbulence, key: "turbulence", title: "Seat belts on (turbulence)", role: Role::Crew, when: "Seat belt sign switched back on in cruise" },
    Script { cue: Cue::Holding, key: "holding", title: "Holding", role: Role::Captain, when: "Flying a full circle in the air (a hold)" },
    Script { cue: Cue::CaptainDescent, key: "captain_descent", title: "Captain's descent announcement", role: Role::Captain, when: "10 nm before the FMC top of descent, or once descending" },
    Script { cue: Cue::PrepareCabinLanding, key: "prepare_cabin_landing", title: "Descent, seat belts on", role: Role::Crew, when: "Descending below about 10,000 ft, or belt sign on in the descent" },
    Script { cue: Cue::CabinDimLanding, key: "cabin_dim_landing", title: "Cabin lights dimmed for landing", role: Role::Crew, when: "Descending at night" },
    Script { cue: Cue::CabinSecureLanding, key: "cabin_secure_landing", title: "Cabin crew, secure the cabin for landing", role: Role::Captain, when: "Below 5,000 ft on the approach" },
    Script { cue: Cue::CrewSeatsLanding, key: "crew_seats_landing", title: "Cabin crew, seats for landing", role: Role::Captain, when: "Gear down or below 2,500 ft" },
    Script { cue: Cue::GoAround, key: "go_around", title: "Go-around", role: Role::Captain, when: "Climbing away from an approach" },
    Script { cue: Cue::AfterLanding, key: "after_landing", title: "Welcome after landing", role: Role::Crew, when: "Slowed below 30 kt after touchdown" },
    Script { cue: Cue::DisarmDoors, key: "disarm_doors", title: "Disarm doors", role: Role::Crew, when: "Parked, parking brake set, engines off" },
    Script { cue: Cue::Disembark, key: "disembark", title: "Disembarking", role: Role::Crew, when: "Belt sign off at the gate, or GSX deboarding starts" },
];

pub fn script(cue: Cue) -> &'static Script {
    SCRIPTS.iter().find(|s| s.cue == cue).expect("every cue has a script")
}

/// Morning, afternoon, evening or night, from seconds since local midnight.
pub fn daypart(secs: f64) -> &'static str {
    match (secs / 3600.0) as i64 % 24 {
        4..=11 => "morning",
        12..=17 => "afternoon",
        18..=21 => "evening",
        _ => "night",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cue_has_one_script() {
        for cue in Cue::ALL {
            assert_eq!(SCRIPTS.iter().filter(|s| s.cue == cue).count(), 1, "{cue:?}");
        }
        assert_eq!(SCRIPTS.len(), Cue::ALL.len());
    }

    #[test]
    fn dayparts() {
        assert_eq!(daypart(7.0 * 3600.0), "morning");
        assert_eq!(daypart(15.0 * 3600.0), "afternoon");
        assert_eq!(daypart(20.0 * 3600.0), "evening");
        assert_eq!(daypart(1.0 * 3600.0), "night");
    }
}
