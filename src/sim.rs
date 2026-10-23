//! The sim thread: connects to MSFS, reads telemetry once a second, runs the
//! flight-phase director and sends announcements to the audio thread.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::airline;
use crate::announcements::{BOARDING_MUSIC, CABIN_AMBIENCE, script};
use crate::announcer::{self, Live, Outcome};
use crate::audio::{Audio, View};
use crate::config::Config;
use crate::packs;
use crate::phase::{Cue, Director, DoorState, GSX_PERFORMING, Gsx, Phase, Telemetry};
use crate::pmdg::{self, Belts, Doors, PmdgData};
use crate::simconnect::{DATATYPE_FLOAT64, DATATYPE_STRING256, PERIOD_SECOND, Recv, SimConnect};

const DEF_TELEMETRY: u32 = 1;
const DEF_TITLE: u32 = 2;
const REQ_TELEMETRY: u32 = 1;
const REQ_TITLE: u32 = 2;
/// Text vars that identify the airline, one definition each so a var the
/// sim doesn't know can't break the others. Request id = definition id.
const TEXT_VARS: &[(u32, &str)] = &[(3, "LIVERY NAME"), (4, "ATC AIRLINE")];
const REQ_LIVERY: u32 = 3;
const REQ_ATC_AIRLINE: u32 = 4;
/// GSX Pro's service states, in their own definition: without GSX they're 0.
const DEF_GSX: u32 = 6;
const REQ_GSX: u32 = 6;
const GSX_VARS: &[&str] = &["L:FSDT_GSX_BOARDING_STATE", "L:FSDT_GSX_DEBOARDING_STATE", "L:FSDT_GSX_DEPARTURE_STATE"];
const REQ_NG3: u32 = 10;
const REQ_777: u32 = 11;
const EVT_PAUSE: u32 = 1;
const EVT_SIM: u32 = 2;
const EVT_FLIGHT_LOADED: u32 = 3;
const RECV_ID_EVENT_FILENAME: u32 = 6;

/// (name, unit), all read as FLOAT64 in this order.
const VARS: &[(&str, &str)] = &[
    ("SIM ON GROUND", "Bool"),
    ("GROUND VELOCITY", "knots"),
    ("PLANE ALTITUDE", "feet"),
    ("PLANE ALT ABOVE GROUND", "feet"),
    ("VERTICAL SPEED", "feet per minute"),
    ("GENERAL ENG COMBUSTION:1", "Bool"),
    ("GENERAL ENG COMBUSTION:2", "Bool"),
    ("GENERAL ENG COMBUSTION:3", "Bool"),
    ("GENERAL ENG COMBUSTION:4", "Bool"),
    ("BRAKE PARKING POSITION", "Bool"),
    ("GEAR HANDLE POSITION", "Bool"),
    ("LIGHT STROBE", "Bool"),
    ("LIGHT LANDING", "Bool"),
    ("VELOCITY BODY Z", "feet per second"),
    ("CABIN SEATBELTS ALERT SWITCH", "Bool"),
    ("INTERACTIVE POINT OPEN:0", "Percent over 100"),
    ("LOCAL TIME", "Seconds"),
    ("AMBIENT TEMPERATURE", "Celsius"),
    ("FLAPS HANDLE INDEX", "Number"),
    ("PLANE HEADING DEGREES MAGNETIC", "Degrees"),
    ("CAMERA STATE", "Number"),
    ("ABSOLUTE TIME", "Seconds"),
];

/// ABSOLUTE TIME counts seconds from 1 January year 1.
const ABSOLUTE_TIME_UNIX_EPOCH: f64 = 62_135_596_800.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Connection {
    NotRunning,
    Connected,
    Error(String),
}

#[derive(Clone)]
pub struct LogLine {
    pub time: String,
    pub text: String,
}

#[derive(Clone)]
pub struct State {
    pub connection: Connection,
    pub aircraft: String,
    pub livery: String,
    pub atc_airline: String,
    pub pmdg: Option<PmdgData>,
    pub telemetry: Telemetry,
    pub phase: Phase,
    pub view: View,
    pub paused: bool,
    pub played: Vec<Cue>,
    pub live: Live,
    pub log: Vec<LogLine>,
    /// Set when the sim quits and the app was started with it.
    pub quit: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            connection: Connection::NotRunning,
            aircraft: String::new(),
            livery: String::new(),
            atc_airline: String::new(),
            pmdg: None,
            telemetry: Telemetry::default(),
            phase: Phase::AtGate,
            view: View::Cabin,
            paused: false,
            played: Vec::new(),
            live: Live::default(),
            log: Vec::new(),
            quit: false,
        }
    }
}

impl State {
    pub fn log(&mut self, text: impl Into<String>) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        // UTC is fine for a log; it only orders events.
        let time = format!("{:02}:{:02}:{:02}Z", secs / 3600 % 24, secs / 60 % 60, secs % 60);
        self.log.push(LogLine { time, text: text.into() });
        if self.log.len() > 300 {
            self.log.remove(0);
        }
    }
}

pub enum Command {
    NewFlight,
}

pub struct Sim {
    pub state: Arc<Mutex<State>>,
    tx: Sender<Command>,
}

impl Sim {
    pub fn start(config: Arc<Mutex<Config>>, audio: Audio, autostarted: bool) -> Sim {
        let state = Arc::new(Mutex::new(State::default()));
        let (tx, rx) = channel();
        let s = state.clone();
        std::thread::Builder::new()
            .name("sim".into())
            .spawn(move || run(s, config, audio, rx, autostarted))
            .expect("spawn sim thread");
        Sim { state, tx }
    }

    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }
}

fn c_string(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

fn f64_at(b: &[u8], i: usize) -> f64 {
    b.get(i * 8..i * 8 + 8).map_or(0.0, |s| f64::from_le_bytes(s.try_into().unwrap()))
}

struct Raw {
    t: Telemetry,
    flaps: f64,
    local_time: f64,
    camera: u32,
    /// Sim date and time (UTC) as Unix seconds.
    sim_unix: f64,
}

fn parse_vars(b: &[u8]) -> Raw {
    let v = |i| f64_at(b, i);
    let on = |i| v(i) > 0.5;
    let on_ground = on(0);
    let local_time = v(16);
    let hour = (local_time / 3600.0) as i64 % 24;
    Raw {
        t: Telemetry {
            on_ground,
            ground_speed: v(1),
            altitude: v(2),
            agl: v(3),
            vertical_speed: v(4),
            heading: v(19),
            engines_running: on(5) || on(6) || on(7) || on(8),
            parking_brake: on(9),
            gear_down: on(10),
            strobe: on(11),
            landing_light: on(12),
            // Rolling backwards on the ground: a tug (or GSX) is pushing us.
            pushback: on_ground && v(13) < -0.3,
            belts: on(14),
            doors: if v(15) > 0.05 { DoorState::Open } else { DoorState::Closed },
            night: !(6..19).contains(&hour),
            ..Default::default()
        },
        flaps: v(18),
        local_time,
        camera: v(20) as u32,
        sim_unix: v(21) - ABSOLUTE_TIME_UNIX_EPOCH,
    }
}

/// CAMERA STATE: 2 is the cockpit; 3 to 10 are outside views (chase,
/// drone, fixed, tower...). Anything else is treated as the cabin.
fn view_of(camera: u32) -> View {
    match camera {
        2 => View::Cockpit,
        3..=10 => View::Outside,
        _ => View::Cabin,
    }
}

/// Scheduled departure passed by more than 10 minutes, judged by the sim's
/// clock. Ignored when the sim's date is nowhere near the plan's.
fn is_late(sim_unix: f64, scheduled: Option<i64>) -> bool {
    scheduled.is_some_and(|s| {
        let late_by = sim_unix - s as f64;
        late_by > 600.0 && late_by < 12.0 * 3600.0
    })
}

/// PMDG data, when present, replaces the generic belt and door state.
fn merge(raw: &Raw, p: Option<&PmdgData>, gsx: Gsx, scheduled: Option<i64>) -> Telemetry {
    let mut t = raw.t.clone();
    t.gsx = gsx;
    t.late_departure = is_late(raw.sim_unix, scheduled);
    if let Some(p) = p {
        t.belts = match p.belts {
            Belts::On => true,
            Belts::Off => false,
            // AUTO lights the sign with gear or flaps out, or low down.
            Belts::Auto => t.gear_down || raw.flaps > 0.5 || t.altitude < 10_300.0,
        };
        t.doors = match p.doors {
            Doors::Open => DoorState::Open,
            Doors::Closed => DoorState::Closed,
            Doors::Armed => DoorState::Armed,
        };
        t.cruise_alt = p.cruise_alt;
        t.distance_to_tod = p.distance_to_tod;
    }
    t
}

fn connect() -> Result<SimConnect, String> {
    let sc = SimConnect::open("Cabin Announcer")?;
    for (name, unit) in VARS {
        sc.add_to_data_definition(DEF_TELEMETRY, name, unit, DATATYPE_FLOAT64)?;
    }
    sc.add_to_data_definition(DEF_TITLE, "TITLE", "", DATATYPE_STRING256)?;
    sc.request_data_on_user(REQ_TELEMETRY, DEF_TELEMETRY, PERIOD_SECOND)?;
    sc.request_data_on_user(REQ_TITLE, DEF_TITLE, PERIOD_SECOND)?;
    for &(id, name) in TEXT_VARS {
        sc.add_to_data_definition(id, name, "", DATATYPE_STRING256)?;
        sc.request_data_on_user(id, id, PERIOD_SECOND)?;
    }
    for name in GSX_VARS {
        sc.add_to_data_definition(DEF_GSX, name, "Number", DATATYPE_FLOAT64)?;
    }
    sc.request_data_on_user(REQ_GSX, DEF_GSX, PERIOD_SECOND)?;
    sc.request_client_area(pmdg::NG3_DATA_NAME, pmdg::NG3_DATA_ID, pmdg::NG3_DATA_DEFINITION, REQ_NG3, pmdg::NG3_DATA_SIZE)?;
    sc.request_client_area(pmdg::T7_DATA_NAME, pmdg::T7_DATA_ID, pmdg::T7_DATA_DEFINITION, REQ_777, pmdg::T7_DATA_SIZE)?;
    sc.subscribe_to_system_event(EVT_PAUSE, "Pause")?;
    sc.subscribe_to_system_event(EVT_SIM, "Sim")?;
    sc.subscribe_to_system_event(EVT_FLIGHT_LOADED, "FlightLoaded")?;
    Ok(sc)
}

fn run(state: Arc<Mutex<State>>, config: Arc<Mutex<Config>>, audio: Audio, rx: Receiver<Command>, autostarted: bool) {
    loop {
        match connect() {
            Ok(sc) => {
                {
                    let mut s = state.lock().unwrap();
                    s.connection = Connection::Connected;
                    s.log("Connected to the sim");
                }
                if !session(&sc, &state, &config, &audio, &rx) {
                    return;
                }
                audio.music(Vec::new());
                audio.ambience(Vec::new());
                let mut s = state.lock().unwrap();
                s.connection = Connection::NotRunning;
                s.aircraft.clear();
                s.livery.clear();
                s.atc_airline.clear();
                s.pmdg = None;
                s.log("Sim closed");
                if autostarted {
                    s.quit = true;
                    return;
                }
            }
            Err(e) => {
                let mut s = state.lock().unwrap();
                s.connection = if e.contains("SimConnect.dll") { Connection::Error(e) } else { Connection::NotRunning };
            }
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

/// Keeps the background layers in step with the phase, only messaging the
/// audio thread when something changes.
#[derive(Default)]
struct Background {
    music: Vec<PathBuf>,
    ambience: Vec<PathBuf>,
}

impl Background {
    fn update(&mut self, audio: &Audio, phase: Phase, gsx: Gsx, pack: Option<&packs::Pack>, config: &Config) {
        let tracks = |folder| packs::background(folder, announcer::source_for(folder, pack, config).as_ref());
        // Boarding music only while GSX is boarding passengers.
        let music = if gsx.boarding == GSX_PERFORMING { tracks(BOARDING_MUSIC) } else { Vec::new() };
        let cabin = matches!(
            phase,
            Phase::DoorsClosed | Phase::Pushback | Phase::TaxiOut | Phase::TakeoffRoll | Phase::Climb | Phase::Cruise | Phase::Descent | Phase::Approach | Phase::Rollout | Phase::TaxiIn
        );
        let ambience = if cabin { tracks(CABIN_AMBIENCE) } else { Vec::new() };
        if music != self.music {
            audio.music(music.clone());
            self.music = music;
        }
        if ambience != self.ambience {
            audio.ambience(ambience.clone());
            self.ambience = ambience;
        }
    }
}

/// Runs until the sim quits (true) or the app closes (false).
fn session(sc: &SimConnect, state: &Mutex<State>, config: &Mutex<Config>, audio: &Audio, rx: &Receiver<Command>) -> bool {
    let mut director = Director::new();
    let mut pmdg_data: Option<PmdgData> = None;
    let mut gsx = Gsx::default();
    let mut background = Background::default();
    let mut view = View::Cabin;
    let mut paused = false;
    let mut running = true;
    // Sim clock: only advances while the sim runs unpaused.
    let mut clock = 0.0f64;
    let mut last_tick = Instant::now();

    loop {
        match rx.try_recv() {
            Ok(Command::NewFlight) => {
                director.new_flight(clock);
                state.lock().unwrap().log("New flight started from the gate");
            }
            Err(TryRecvError::Disconnected) => return false,
            Err(TryRecvError::Empty) => {}
        }
        let now = Instant::now();
        if running && !paused {
            clock += (now - last_tick).as_secs_f64();
        }
        last_tick = now;

        let mut got_any = false;
        while let Some(msg) = sc.next() {
            got_any = true;
            match msg {
                Recv::Quit => return true,
                Recv::Event { id: EVT_PAUSE, data } => {
                    paused = data != 0;
                    audio.pause(paused);
                    state.lock().unwrap().paused = paused;
                }
                Recv::Event { id: EVT_SIM, data } => running = data != 0,
                Recv::Other(RECV_ID_EVENT_FILENAME) => {
                    // A new flight was loaded: start over and re-detect the phase.
                    director = Director::new();
                    state.lock().unwrap().log("Flight loaded");
                }
                Recv::ObjectData { request_id: REQ_GSX, data } => {
                    let v = |i| f64_at(&data, i).clamp(0.0, 255.0) as u8;
                    let next = Gsx { boarding: v(0), deboarding: v(1), departure: v(2) };
                    if next != gsx && next.boarding + next.deboarding + next.departure > 0 && gsx == Gsx::default() {
                        state.lock().unwrap().log("GSX detected");
                    }
                    gsx = next;
                }
                Recv::ObjectData { request_id: id @ (REQ_LIVERY | REQ_ATC_AIRLINE), data } => {
                    let text = c_string(&data);
                    let mut s = state.lock().unwrap();
                    if id == REQ_LIVERY { s.livery = text } else { s.atc_airline = text }
                }
                Recv::ObjectData { request_id: REQ_TITLE, data } => {
                    let title = c_string(&data);
                    let mut s = state.lock().unwrap();
                    if s.aircraft != title {
                        if !title.is_empty() {
                            s.log(format!("Aircraft: {title}"));
                        }
                        s.aircraft = title;
                        pmdg_data = None;
                        s.pmdg = None;
                    }
                }
                Recv::ClientData { request_id, data } if request_id == REQ_NG3 || request_id == REQ_777 => {
                    let p = if request_id == REQ_NG3 { pmdg::parse_ng3(&data) } else { pmdg::parse_777(&data) };
                    pmdg_data = Some(p.clone());
                    state.lock().unwrap().pmdg = Some(p);
                }
                Recv::ObjectData { request_id: REQ_TELEMETRY, data } => {
                    let raw = parse_vars(&data);
                    let v = view_of(raw.camera);
                    if v != view {
                        audio.view(v);
                        view = v;
                        state.lock().unwrap().view = v;
                    }
                    if !running || paused {
                        continue;
                    }
                    let cfg = config.lock().unwrap().clone();
                    let t = merge(&raw, pmdg_data.as_ref(), gsx, cfg.flight.scheduled_departure);
                    let cues = director.update(&t, clock);
                    let mut s = state.lock().unwrap();
                    let live = Live {
                        local_time: Some(raw.local_time),
                        airline: airline::detect(&airline::Clues {
                            simbrief_icao: &cfg.flight.airline_icao,
                            livery: &s.livery,
                            atc_airline: &s.atc_airline,
                            title: &s.aircraft,
                        }),
                    };
                    if live.airline != s.live.airline {
                        let msg = match &live.airline {
                            Some(a) => format!("Airline: {} ({}, from the {})", if a.name.is_empty() { &a.icao } else { &a.name }, a.icao, a.source),
                            None => "Airline: not recognised".to_string(),
                        };
                        s.log(msg);
                    }
                    let pack = announcer::pack_in_use(&cfg, &live);
                    background.update(audio, director.phase(), gsx, pack.as_ref(), &cfg);
                    s.phase = director.phase();
                    s.telemetry = t;
                    s.live = live.clone();
                    s.played = Cue::ALL.into_iter().filter(|&c| director.has_played(c)).collect();
                    for cue in cues {
                        let title = script(cue).title;
                        let phase = director.phase().label();
                        match announcer::item(cue, &cfg, &live, pack.as_ref(), false) {
                            Outcome::Play(item) => {
                                s.log(format!("{phase}: {title}"));
                                audio.play(item);
                            }
                            Outcome::SwitchedOff => s.log(format!("{phase}: {title} (switched off)")),
                            Outcome::NoRecording => {
                                let from = pack.as_ref().map_or("no pack in use".to_string(), |p| format!("not in {}", p.title()));
                                s.log(format!("{phase}: {title} (silent, {from})"));
                            }
                        }
                    }
                }
                Recv::Exception { code, send_id, index } => {
                    state.lock().unwrap().log(format!("SimConnect exception {code} (send {send_id}, index {index})"));
                }
                _ => {}
            }
        }
        if !got_any {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lateness_needs_a_plausible_date() {
        let sched = Some(1_000_000_000);
        assert!(!is_late(1_000_000_300.0, sched));
        assert!(is_late(1_000_000_700.0, sched));
        assert!(!is_late(1_000_000_000.0 + 2.0 * 86400.0, sched), "sim set to another day");
        assert!(!is_late(1_000_000_700.0, None));
    }

    #[test]
    fn cameras() {
        assert_eq!(view_of(2), View::Cockpit);
        assert_eq!(view_of(3), View::Outside);
        assert_eq!(view_of(0), View::Cabin);
    }
}
