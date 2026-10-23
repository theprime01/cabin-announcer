//! Works out the phase of flight from sim telemetry and decides when each
//! announcement ("cue") should play. Pure logic, driven by `Director::update`.

use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DoorState {
    /// The aircraft doesn't report its doors.
    #[default]
    Unknown,
    Open,
    Closed,
    Armed,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Telemetry {
    pub on_ground: bool,
    /// Knots.
    pub ground_speed: f64,
    /// Feet MSL.
    pub altitude: f64,
    /// Feet above ground.
    pub agl: f64,
    /// Feet per minute.
    pub vertical_speed: f64,
    pub engines_running: bool,
    pub parking_brake: bool,
    pub gear_down: bool,
    pub strobe: bool,
    pub landing_light: bool,
    pub pushback: bool,
    /// Fasten seat belts sign lit.
    pub belts: bool,
    pub doors: DoorState,
    /// Feet, from the FMC when the aircraft reports it.
    pub cruise_alt: Option<f64>,
    /// Nautical miles to top of descent, from the FMC (0 once passed).
    pub distance_to_tod: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    AtGate,
    Boarding,
    DoorsClosed,
    Pushback,
    TaxiOut,
    TakeoffRoll,
    Climb,
    Cruise,
    Descent,
    Approach,
    Rollout,
    TaxiIn,
    Arrived,
    Turnaround,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::AtGate => "At the gate",
            Phase::Boarding => "Boarding",
            Phase::DoorsClosed => "Doors closed",
            Phase::Pushback => "Pushback",
            Phase::TaxiOut => "Taxi out",
            Phase::TakeoffRoll => "Takeoff",
            Phase::Climb => "Climb",
            Phase::Cruise => "Cruise",
            Phase::Descent => "Descent",
            Phase::Approach => "Approach",
            Phase::Rollout => "Landing rollout",
            Phase::TaxiIn => "Taxi in",
            Phase::Arrived => "Arrived at the gate",
            Phase::Turnaround => "Turnaround",
        }
    }
}

/// Every automatic announcement, in the order they normally play.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cue {
    BoardingWelcome,
    ArmDoors,
    CaptainWelcome,
    SafetyDemo,
    CrewSeatsTakeoff,
    BeltsOffClimb,
    CaptainCruise,
    Turbulence,
    CaptainDescent,
    PrepareCabinLanding,
    CrewSeatsLanding,
    GoAround,
    AfterLanding,
    DisarmDoors,
    Disembark,
}

impl Cue {
    pub const ALL: [Cue; 15] = [
        Cue::BoardingWelcome,
        Cue::ArmDoors,
        Cue::CaptainWelcome,
        Cue::SafetyDemo,
        Cue::CrewSeatsTakeoff,
        Cue::BeltsOffClimb,
        Cue::CaptainCruise,
        Cue::Turbulence,
        Cue::CaptainDescent,
        Cue::PrepareCabinLanding,
        Cue::CrewSeatsLanding,
        Cue::GoAround,
        Cue::AfterLanding,
        Cue::DisarmDoors,
        Cue::Disembark,
    ];
}

/// Tracks how long a condition has been true.
#[derive(Default, Clone, Copy)]
struct Hold(Option<f64>);

impl Hold {
    fn update(&mut self, cond: bool, now: f64) {
        if !cond {
            self.0 = None;
        } else if self.0.is_none() {
            self.0 = Some(now);
        }
    }
    fn held(&self, now: f64, secs: f64) -> bool {
        self.0.is_some_and(|t| now - t >= secs)
    }
}

#[derive(Default)]
struct Holds {
    doors_open: Hold,
    doors_shut: Hold,
    moving: Hold,
    taxiing: Hold,
    airborne: Hold,
    grounded: Hold,
    level: Hold,
    descending: Hold,
    climbing: Hold,
    slow: Hold,
    parked: Hold,
}

const TURNAROUND_SECS: f64 = 600.0;
const TURBULENCE_COOLDOWN_SECS: f64 = 600.0;

pub struct Director {
    phase: Phase,
    phase_since: f64,
    done: HashSet<Cue>,
    holds: Holds,
    started: bool,
    last: Telemetry,
    /// The belt sign was lit at some point this flight, so the aircraft reports it.
    belts_seen_on: bool,
    max_altitude: f64,
    min_approach_agl: f64,
    last_turbulence: Option<f64>,
}

impl Default for Director {
    fn default() -> Self {
        Self::new()
    }
}

impl Director {
    pub fn new() -> Director {
        Director {
            phase: Phase::AtGate,
            phase_since: 0.0,
            done: HashSet::new(),
            holds: Holds::default(),
            started: false,
            last: Telemetry::default(),
            belts_seen_on: false,
            max_altitude: 0.0,
            min_approach_agl: f64::MAX,
            last_turbulence: None,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn has_played(&self, cue: Cue) -> bool {
        self.done.contains(&cue)
    }

    /// Starts a fresh flight from the gate (the "New flight" button).
    pub fn new_flight(&mut self, now: f64) {
        self.done.clear();
        self.belts_seen_on = false;
        self.max_altitude = 0.0;
        self.min_approach_agl = f64::MAX;
        self.last_turbulence = None;
        self.set_phase(Phase::AtGate, now);
    }

    fn set_phase(&mut self, phase: Phase, now: f64) {
        if self.phase != phase {
            self.phase = phase;
            self.phase_since = now;
        }
    }

    fn in_phase_for(&self, now: f64) -> f64 {
        now - self.phase_since
    }

    fn cue(&mut self, cue: Cue, out: &mut Vec<Cue>) {
        if self.done.insert(cue) {
            out.push(cue);
        }
    }

    fn skip(&mut self, cues: &[Cue]) {
        self.done.extend(cues.iter().copied());
    }

    /// Picks a sensible starting phase when the app connects mid-flight, and
    /// marks the announcements that would already have happened as played.
    fn start(&mut self, t: &Telemetry, now: f64) {
        use Cue::*;
        self.started = true;
        self.max_altitude = t.altitude;
        let departure = [BoardingWelcome, ArmDoors, CaptainWelcome, SafetyDemo, CrewSeatsTakeoff];
        let phase = if !t.on_ground {
            self.skip(&departure);
            self.belts_seen_on = t.belts;
            if t.agl < 4000.0 && t.vertical_speed < -300.0 {
                self.skip(&[BeltsOffClimb, CaptainCruise, CaptainDescent, PrepareCabinLanding]);
                Phase::Approach
            } else if t.vertical_speed < -500.0 {
                self.skip(&[BeltsOffClimb, CaptainCruise, CaptainDescent]);
                Phase::Descent
            } else if t.vertical_speed > 500.0 {
                Phase::Climb
            } else {
                self.skip(&[BeltsOffClimb]);
                Phase::Cruise
            }
        } else if t.engines_running {
            self.skip(&departure);
            if t.ground_speed > 40.0 { Phase::TakeoffRoll } else { Phase::TaxiOut }
        } else {
            Phase::AtGate
        };
        self.phase = phase;
        self.phase_since = now;
    }

    pub fn update(&mut self, t: &Telemetry, now: f64) -> Vec<Cue> {
        use Cue::*;
        let mut out = Vec::new();
        if !self.started {
            self.start(t, now);
            self.last = t.clone();
            return out;
        }

        let prev = std::mem::replace(&mut self.last, t.clone());
        let doors_open = t.doors == DoorState::Open;
        let doors_shut = matches!(t.doors, DoorState::Closed | DoorState::Armed);
        let moving = t.pushback || t.ground_speed > 1.0;
        let belts_on_now = t.belts && !prev.belts;
        let belts_off_now = !t.belts && prev.belts;
        if t.belts {
            self.belts_seen_on = true;
        }
        if !t.on_ground {
            self.max_altitude = self.max_altitude.max(t.altitude);
        }

        let h = &mut self.holds;
        h.doors_open.update(doors_open, now);
        h.doors_shut.update(doors_shut, now);
        h.moving.update(moving, now);
        h.taxiing.update(t.engines_running && !t.pushback && t.ground_speed > 3.0, now);
        h.airborne.update(!t.on_ground && t.agl > 50.0, now);
        h.grounded.update(t.on_ground, now);
        h.level.update(!t.on_ground && t.vertical_speed.abs() < 300.0, now);
        h.descending.update(!t.on_ground && t.vertical_speed < -800.0, now);
        h.climbing.update(!t.on_ground && t.vertical_speed > 1000.0, now);
        h.slow.update(t.on_ground && t.ground_speed < 30.0, now);
        h.parked.update(t.on_ground && t.ground_speed < 1.0 && t.parking_brake && !t.engines_running, now);
        let h = &self.holds;
        let (doors_open_5, doors_shut_5) = (h.doors_open.held(now, 5.0), h.doors_shut.held(now, 5.0));
        let (moving_2, taxiing_3, airborne_2) = (h.moving.held(now, 2.0), h.taxiing.held(now, 3.0), h.airborne.held(now, 2.0));
        let grounded_2 = h.grounded.held(now, 2.0);
        let level_cruise = match t.cruise_alt {
            Some(c) if t.altitude > c - 500.0 => h.level.held(now, 30.0),
            _ => t.altitude > 8000.0 && h.level.held(now, 90.0),
        };
        let descending_30 = h.descending.held(now, 30.0);
        let (climbing_8, slow_5, parked_5) = (h.climbing.held(now, 8.0), h.slow.held(now, 5.0), h.parked.held(now, 5.0));

        match self.phase {
            Phase::AtGate | Phase::Boarding => {
                if self.phase == Phase::AtGate && doors_open_5 {
                    self.set_phase(Phase::Boarding, now);
                    self.cue(BoardingWelcome, &mut out);
                } else if self.phase == Phase::Boarding && doors_shut_5 {
                    self.set_phase(Phase::DoorsClosed, now);
                    self.cue(ArmDoors, &mut out);
                } else if t.engines_running || moving_2 {
                    // Aircraft without door data: departing is the first sign.
                    self.set_phase(Phase::DoorsClosed, now);
                    self.cue(ArmDoors, &mut out);
                }
            }
            Phase::DoorsClosed => {
                if doors_open_5 {
                    self.set_phase(Phase::Boarding, now);
                } else if moving_2 {
                    self.cue(CaptainWelcome, &mut out);
                    self.cue(SafetyDemo, &mut out);
                    let next = if taxiing_3 { Phase::TaxiOut } else { Phase::Pushback };
                    self.set_phase(next, now);
                } else if self.in_phase_for(now) >= 20.0 {
                    self.cue(CaptainWelcome, &mut out);
                }
            }
            Phase::Pushback => {
                if taxiing_3 {
                    self.set_phase(Phase::TaxiOut, now);
                }
            }
            Phase::TaxiOut => {
                self.cue(SafetyDemo, &mut out);
                // Strobes or landing lights coming on means entering the runway.
                let lights_on = (t.strobe && !prev.strobe) || (t.landing_light && !prev.landing_light);
                if lights_on && self.in_phase_for(now) > 10.0 {
                    self.cue(CrewSeatsTakeoff, &mut out);
                }
                if t.on_ground && t.ground_speed > 40.0 {
                    self.cue(CrewSeatsTakeoff, &mut out);
                    self.set_phase(Phase::TakeoffRoll, now);
                }
            }
            Phase::TakeoffRoll => {
                if airborne_2 {
                    self.set_phase(Phase::Climb, now);
                } else if slow_5 {
                    // Rejected takeoff.
                    self.set_phase(Phase::TaxiOut, now);
                }
            }
            Phase::Climb => {
                let belts_off = if self.belts_seen_on {
                    !t.belts && t.agl > 3000.0
                } else {
                    t.agl > 10000.0 && self.in_phase_for(now) > 120.0
                };
                if belts_off {
                    self.cue(BeltsOffClimb, &mut out);
                }
                if t.gear_down && t.agl < 4000.0 && t.vertical_speed < -300.0 {
                    // Back on approach after a go-around or a circuit.
                    self.min_approach_agl = t.agl;
                    self.set_phase(Phase::Approach, now);
                } else if level_cruise {
                    self.set_phase(Phase::Cruise, now);
                } else if descending_30 && t.altitude < self.max_altitude - 2000.0 {
                    self.set_phase(Phase::Descent, now);
                }
            }
            Phase::Cruise => {
                if self.belts_seen_on && belts_off_now {
                    self.cue(BeltsOffClimb, &mut out);
                }
                if self.in_phase_for(now) >= 60.0 {
                    self.cue(CaptainCruise, &mut out);
                }
                let since_turbulence = self.last_turbulence.map_or(f64::MAX, |l| now - l);
                if belts_on_now && self.done.contains(&BeltsOffClimb) && since_turbulence > TURBULENCE_COOLDOWN_SECS {
                    // Re-armed so it can play again on the next encounter.
                    self.done.remove(&Turbulence);
                    self.cue(Turbulence, &mut out);
                    self.last_turbulence = Some(now);
                }
                if t.distance_to_tod.is_some_and(|d| d > 0.0 && d <= 10.0) {
                    self.cue(CaptainCruise, &mut out);
                    self.cue(CaptainDescent, &mut out);
                }
                if descending_30 {
                    self.cue(CaptainDescent, &mut out);
                    self.set_phase(Phase::Descent, now);
                }
            }
            Phase::Descent => {
                self.cue(CaptainDescent, &mut out);
                let low = t.agl < 6000.0 || (t.altitude < 11000.0 && t.agl < 10000.0);
                if low || (belts_on_now && self.max_altitude > 15000.0) {
                    self.cue(PrepareCabinLanding, &mut out);
                }
                if t.agl < 4000.0 || t.gear_down {
                    self.cue(PrepareCabinLanding, &mut out);
                    self.min_approach_agl = t.agl;
                    self.set_phase(Phase::Approach, now);
                }
            }
            Phase::Approach => {
                self.min_approach_agl = self.min_approach_agl.min(t.agl);
                if t.gear_down || t.agl < 2500.0 {
                    self.cue(CrewSeatsLanding, &mut out);
                }
                if grounded_2 {
                    self.set_phase(Phase::Rollout, now);
                } else if climbing_8 && self.min_approach_agl < 1500.0 && t.agl > self.min_approach_agl + 400.0 {
                    self.done.remove(&GoAround);
                    self.cue(GoAround, &mut out);
                    // The crew sit down again for the next approach.
                    self.done.remove(&CrewSeatsLanding);
                    self.min_approach_agl = t.agl;
                    self.set_phase(Phase::Climb, now);
                    self.skip(&[BeltsOffClimb, CaptainCruise, CaptainDescent, PrepareCabinLanding]);
                }
            }
            Phase::Rollout => {
                if airborne_2 {
                    // Touch and go.
                    self.set_phase(Phase::Climb, now);
                } else if slow_5 {
                    self.cue(AfterLanding, &mut out);
                    self.set_phase(Phase::TaxiIn, now);
                }
            }
            Phase::TaxiIn => {
                if parked_5 {
                    self.cue(DisarmDoors, &mut out);
                    self.set_phase(Phase::Arrived, now);
                }
            }
            Phase::Arrived => {
                let belts_off = if self.belts_seen_on { !t.belts } else { self.in_phase_for(now) >= 20.0 };
                if belts_off && self.in_phase_for(now) >= 5.0 {
                    self.cue(Disembark, &mut out);
                    self.set_phase(Phase::Turnaround, now);
                }
            }
            Phase::Turnaround => {
                if self.in_phase_for(now) >= TURNAROUND_SECS {
                    self.new_flight(now);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds one telemetry sample per second for `secs` seconds.
    struct Sim {
        d: Director,
        t: Telemetry,
        now: f64,
        cues: Vec<Cue>,
    }

    impl Sim {
        fn new(t: Telemetry) -> Sim {
            let mut s = Sim { d: Director::new(), t, now: 0.0, cues: Vec::new() };
            s.run(1);
            s
        }
        fn run(&mut self, secs: u32) {
            for _ in 0..secs {
                self.cues.extend(self.d.update(&self.t, self.now));
                self.now += 1.0;
            }
        }
        fn set(&mut self, f: impl FnOnce(&mut Telemetry), secs: u32) {
            f(&mut self.t);
            self.run(secs);
        }
    }

    fn at_gate() -> Telemetry {
        Telemetry {
            on_ground: true,
            parking_brake: true,
            belts: false,
            doors: DoorState::Closed,
            ..Default::default()
        }
    }

    fn fly(s: &mut Sim) {
        // Boarding, doors closed, belts on.
        s.set(|t| t.doors = DoorState::Open, 10);
        s.set(|t| t.belts = true, 60);
        s.set(|t| t.doors = DoorState::Armed, 30);
        // Pushback and engine start.
        s.set(|t| { t.pushback = true; t.parking_brake = false; t.ground_speed = 2.0; }, 60);
        s.set(|t| { t.pushback = false; t.ground_speed = 0.0; t.engines_running = true; }, 30);
        // Taxi, then lights on at the runway.
        s.set(|t| t.ground_speed = 15.0, 120);
        s.set(|t| { t.ground_speed = 0.0; t.strobe = true; t.landing_light = true; }, 20);
        // Takeoff and climb.
        s.set(|t| t.ground_speed = 140.0, 30);
        s.set(|t| { t.on_ground = false; t.agl = 500.0; t.altitude = 600.0; t.vertical_speed = 2500.0; t.gear_down = false; }, 10);
        s.set(|t| { t.agl = 12000.0; t.altitude = 12000.0; t.belts = false; }, 60);
        // Cruise.
        s.set(|t| { t.agl = 35000.0; t.altitude = 35000.0; t.vertical_speed = 0.0; }, 200);
        // Turbulence.
        s.set(|t| t.belts = true, 10);
        s.set(|t| t.belts = false, 10);
        // Descent.
        s.set(|t| t.vertical_speed = -2000.0, 40);
        s.set(|t| { t.altitude = 9000.0; t.agl = 8000.0; t.belts = true; }, 30);
        s.set(|t| { t.altitude = 3000.0; t.agl = 3000.0; t.gear_down = true; }, 30);
        // Touchdown, rollout, taxi in, park.
        s.set(|t| { t.on_ground = true; t.agl = 0.0; t.altitude = 100.0; t.vertical_speed = 0.0; t.ground_speed = 120.0; }, 10);
        s.set(|t| t.ground_speed = 15.0, 60);
        s.set(|t| { t.ground_speed = 0.0; t.parking_brake = true; t.engines_running = false; t.strobe = false; t.landing_light = false; }, 10);
        s.set(|t| t.belts = false, 10);
    }

    #[test]
    fn full_flight_plays_every_cue_in_order() {
        let mut s = Sim::new(at_gate());
        fly(&mut s);
        use Cue::*;
        assert_eq!(
            s.cues,
            vec![
                BoardingWelcome,
                ArmDoors,
                CaptainWelcome,
                SafetyDemo,
                CrewSeatsTakeoff,
                BeltsOffClimb,
                CaptainCruise,
                Turbulence,
                CaptainDescent,
                PrepareCabinLanding,
                CrewSeatsLanding,
                AfterLanding,
                DisarmDoors,
                Disembark,
            ]
        );
        assert_eq!(s.d.phase(), Phase::Turnaround);
    }

    #[test]
    fn turnaround_starts_the_next_flight() {
        let mut s = Sim::new(at_gate());
        fly(&mut s);
        s.run(TURNAROUND_SECS as u32 + 1);
        assert_eq!(s.d.phase(), Phase::AtGate);
        let before = s.cues.len();
        s.set(|t| t.doors = DoorState::Open, 10);
        assert_eq!(&s.cues[before..], &[Cue::BoardingWelcome]);
    }

    #[test]
    fn connecting_in_cruise_skips_the_departure() {
        let mut s = Sim::new(Telemetry { altitude: 37000.0, agl: 36000.0, ..Default::default() });
        assert_eq!(s.d.phase(), Phase::Cruise);
        s.run(70);
        assert_eq!(s.cues, vec![Cue::CaptainCruise]);
    }

    #[test]
    fn go_around_rearms_crew_seats() {
        let mut s = Sim::new(Telemetry { altitude: 3000.0, agl: 2000.0, vertical_speed: -700.0, gear_down: true, ..Default::default() });
        assert_eq!(s.d.phase(), Phase::Approach);
        s.run(5);
        s.set(|t| { t.agl = 200.0; t.altitude = 300.0; }, 5);
        s.set(|t| { t.agl = 900.0; t.vertical_speed = 2000.0; }, 10);
        assert!(s.cues.contains(&Cue::GoAround));
        s.set(|t| { t.vertical_speed = -700.0; t.agl = 2000.0; }, 40);
        let seats = s.cues.iter().filter(|&&c| c == Cue::CrewSeatsLanding).count();
        assert_eq!(seats, 2);
    }

    #[test]
    fn aircraft_without_doors_or_belts_still_flies_through() {
        let mut s = Sim::new(Telemetry { on_ground: true, parking_brake: true, ..Default::default() });
        s.set(|t| { t.engines_running = true; t.parking_brake = false; }, 30);
        s.set(|t| t.ground_speed = 15.0, 60);
        s.set(|t| t.ground_speed = 140.0, 10);
        s.set(|t| { t.on_ground = false; t.agl = 11000.0; t.altitude = 11000.0; t.vertical_speed = 2000.0; }, 150);
        use Cue::*;
        assert_eq!(s.cues, vec![ArmDoors, CaptainWelcome, SafetyDemo, CrewSeatsTakeoff, BeltsOffClimb]);
    }
}
