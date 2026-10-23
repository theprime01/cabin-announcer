//! Reads the few fields we need from the PMDG 737 (NG3) and 777 SDK data
//! broadcasts. Offsets come from tools/pmdg-offsets.ps1, which compiles the
//! SDK headers that ship with the aircraft; re-run it after a PMDG SDK update.
//!
//! The broadcast must be enabled in the aircraft's options ini:
//! `[SDK]` / `EnableDataBroadcast=1`.

pub const NG3_DATA_NAME: &str = "PMDG_NG3_Data";
pub const NG3_DATA_ID: u32 = 0x4E47_3331;
pub const NG3_DATA_DEFINITION: u32 = 0x4E47_3332;
pub const NG3_DATA_SIZE: u32 = 916;

pub const T7_DATA_NAME: &str = "PMDG_777X_Data";
pub const T7_DATA_ID: u32 = 0x504D_4447;
pub const T7_DATA_DEFINITION: u32 = 0x504D_4448;
pub const T7_DATA_SIZE: u32 = 684;

mod ng3 {
    pub const FASTEN_BELTS: usize = 219;
    pub const DOOR_FWD_ENTRY: usize = 344;
    pub const DOOR_FWD_SERVICE: usize = 345;
    pub const DOOR_AIRSTAIR: usize = 346;
    pub const DOOR_AFT_ENTRY: usize = 354;
    pub const DOOR_AFT_SERVICE: usize = 355;
    pub const FMC_CRUISE_ALT: usize = 626;
    pub const FMC_DISTANCE_TO_TOD: usize = 636;
    pub const FMC_DISTANCE_TO_DEST: usize = 640;
    pub const FMC_FLIGHT_NUMBER: usize = 644;
}

mod t7 {
    pub const SEAT_BELTS: usize = 99;
    pub const DOOR_STATE: usize = 467;
    pub const FMC_CRUISE_ALT: usize = 558;
    pub const FMC_DISTANCE_TO_TOD: usize = 568;
    pub const FMC_DISTANCE_TO_DEST: usize = 572;
    pub const FMC_FLIGHT_NUMBER: usize = 576;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    B737,
    B777,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Belts {
    Off,
    Auto,
    On,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doors {
    /// At least one passenger door is open (or opening/closing).
    Open,
    Closed,
    /// All passenger doors closed and armed (777 only reports this).
    Armed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PmdgData {
    pub model: Model,
    pub belts: Belts,
    pub doors: Doors,
    /// Feet; None when not entered in the FMC.
    pub cruise_alt: Option<f64>,
    /// Nautical miles; None when not available, Some(0.0) once passed.
    pub distance_to_tod: Option<f64>,
    pub distance_to_dest: Option<f64>,
    pub flight_number: String,
}

fn u8_at(b: &[u8], at: usize) -> u8 {
    b.get(at).copied().unwrap_or(0)
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    b.get(at..at + 2).map_or(0, |s| u16::from_le_bytes(s.try_into().unwrap()))
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    b.get(at..at + 4).map_or(-1.0, |s| f32::from_le_bytes(s.try_into().unwrap()))
}

fn text_at(b: &[u8], at: usize, len: usize) -> String {
    let s = b.get(at..at + len).unwrap_or_default();
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..end]).trim().to_string()
}

fn belts(v: u8) -> Belts {
    match v {
        0 => Belts::Off,
        1 => Belts::Auto,
        _ => Belts::On,
    }
}

fn fmc(b: &[u8], cruise: usize, tod: usize, dest: usize) -> (Option<f64>, Option<f64>, Option<f64>) {
    let cruise = u16_at(b, cruise);
    let tod = f32_at(b, tod);
    let dest = f32_at(b, dest);
    (
        (cruise > 0).then_some(cruise as f64),
        (tod >= 0.0).then_some(tod as f64),
        (dest >= 0.0).then_some(dest as f64),
    )
}

pub fn parse_ng3(b: &[u8]) -> PmdgData {
    use ng3::*;
    let open = [DOOR_FWD_ENTRY, DOOR_FWD_SERVICE, DOOR_AIRSTAIR, DOOR_AFT_ENTRY, DOOR_AFT_SERVICE]
        .iter()
        .any(|&o| u8_at(b, o) != 0);
    let (cruise_alt, distance_to_tod, distance_to_dest) = fmc(b, FMC_CRUISE_ALT, FMC_DISTANCE_TO_TOD, FMC_DISTANCE_TO_DEST);
    PmdgData {
        model: Model::B737,
        belts: belts(u8_at(b, FASTEN_BELTS)),
        doors: if open { Doors::Open } else { Doors::Closed },
        cruise_alt,
        distance_to_tod,
        distance_to_dest,
        flight_number: text_at(b, FMC_FLIGHT_NUMBER, 9),
    }
}

pub fn parse_777(b: &[u8]) -> PmdgData {
    use t7::*;
    // Entries 0..=9 are passenger doors: 0 open, 1 closed, 2 closed and armed,
    // 3 closing, 4 opening.
    let states: Vec<u8> = (0..10).map(|i| u8_at(b, DOOR_STATE + i)).collect();
    let doors = if states.iter().any(|&s| s == 0 || s >= 3) {
        Doors::Open
    } else if states.iter().all(|&s| s == 2) {
        Doors::Armed
    } else {
        Doors::Closed
    };
    let (cruise_alt, distance_to_tod, distance_to_dest) = fmc(b, FMC_CRUISE_ALT, FMC_DISTANCE_TO_TOD, FMC_DISTANCE_TO_DEST);
    PmdgData {
        model: Model::B777,
        belts: belts(u8_at(b, SEAT_BELTS)),
        doors,
        cruise_alt,
        distance_to_tod,
        distance_to_dest,
        flight_number: text_at(b, FMC_FLIGHT_NUMBER, 9),
    }
}

/// The PMDG "work" folders for MSFS 2024 (Steam and Microsoft Store installs).
fn work_dirs() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Some(a) = std::env::var_os("APPDATA") {
        roots.push(std::path::PathBuf::from(a).join("Microsoft Flight Simulator 2024"));
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        roots.push(std::path::PathBuf::from(l).join(r"Packages\Microsoft.Limitless_8wekyb3d8bbwe\LocalState"));
    }
    let mut dirs = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root.join(r"WASM\MSFS2024")) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with("pmdg-aircraft-") && !name.contains("liveries") {
                dirs.push(e.path().join("work"));
            }
        }
    }
    dirs
}

/// Adds `EnableDataBroadcast=1` to the [SDK] section of an options ini.
fn with_broadcast(ini: &str) -> String {
    let mut out = Vec::new();
    let mut in_sdk = false;
    let mut done = false;
    for line in ini.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_sdk && !done {
                out.push("EnableDataBroadcast=1".to_string());
                done = true;
            }
            in_sdk = trimmed.eq_ignore_ascii_case("[SDK]");
        } else if in_sdk && trimmed.to_ascii_lowercase().starts_with("enabledatabroadcast") {
            out.push("EnableDataBroadcast=1".to_string());
            done = true;
            continue;
        }
        out.push(line.to_string());
    }
    if in_sdk && !done {
        out.push("EnableDataBroadcast=1".to_string());
    } else if !done {
        out.push(String::new());
        out.push("[SDK]".to_string());
        out.push("EnableDataBroadcast=1".to_string());
    }
    out.join("\r\n") + "\r\n"
}

/// Turns on the SDK broadcast for every installed PMDG aircraft. Returns the
/// files changed; the aircraft needs reloading afterwards.
pub fn enable_broadcast() -> Result<Vec<std::path::PathBuf>, String> {
    let mut changed = Vec::new();
    for dir in work_dirs() {
        let model = dir.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let existing: Vec<_> = std::fs::read_dir(&dir)
            .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with("_Options.ini")).collect())
            .unwrap_or_default();
        let files = if existing.is_empty() {
            let name = if model.contains("77") { "777_Options.ini" } else { "737_Options.ini" };
            vec![dir.join(name)]
        } else {
            existing
        };
        for f in files {
            let old = std::fs::read_to_string(&f).unwrap_or_default();
            let new = with_broadcast(&old);
            if new.replace("\r\n", "\n").trim() != old.replace("\r\n", "\n").trim() {
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                std::fs::write(&f, new).map_err(|e| format!("{}: {e}", f.display()))?;
                changed.push(f);
            }
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_ini_edit() {
        assert_eq!(with_broadcast(""), "\r\n[SDK]\r\nEnableDataBroadcast=1\r\n");
        assert_eq!(with_broadcast("[SDK]\nEnableDataBroadcast=0\n[X]\na=1"), "[SDK]\r\nEnableDataBroadcast=1\r\n[X]\r\na=1\r\n");
        assert_eq!(with_broadcast("[A]\nb=2\n[SDK]\nEnableCDUBroadcast.0=1"), "[A]\r\nb=2\r\n[SDK]\r\nEnableCDUBroadcast.0=1\r\nEnableDataBroadcast=1\r\n");
        let on = "[SDK]\r\nEnableDataBroadcast=1\r\n";
        assert_eq!(with_broadcast(on), on);
    }

    #[test]
    fn ng3_fields() {
        let mut b = vec![0u8; NG3_DATA_SIZE as usize];
        b[ng3::FASTEN_BELTS] = 2;
        b[ng3::DOOR_AFT_ENTRY] = 1;
        b[ng3::FMC_CRUISE_ALT..ng3::FMC_CRUISE_ALT + 2].copy_from_slice(&35000u16.to_le_bytes());
        b[ng3::FMC_DISTANCE_TO_TOD..ng3::FMC_DISTANCE_TO_TOD + 4].copy_from_slice(&(-1.0f32).to_le_bytes());
        b[ng3::FMC_DISTANCE_TO_DEST..ng3::FMC_DISTANCE_TO_DEST + 4].copy_from_slice(&412.5f32.to_le_bytes());
        b[ng3::FMC_FLIGHT_NUMBER..ng3::FMC_FLIGHT_NUMBER + 6].copy_from_slice(b"BAW117");
        let d = parse_ng3(&b);
        assert_eq!(d.belts, Belts::On);
        assert_eq!(d.doors, Doors::Open);
        assert_eq!(d.cruise_alt, Some(35000.0));
        assert_eq!(d.distance_to_tod, None);
        assert_eq!(d.distance_to_dest, Some(412.5));
        assert_eq!(d.flight_number, "BAW117");
    }

    #[test]
    fn t7_doors() {
        let mut b = vec![0u8; T7_DATA_SIZE as usize];
        b[t7::DOOR_STATE..t7::DOOR_STATE + 10].fill(2);
        assert_eq!(parse_777(&b).doors, Doors::Armed);
        b[t7::DOOR_STATE + 3] = 1;
        assert_eq!(parse_777(&b).doors, Doors::Closed);
        b[t7::DOOR_STATE] = 4;
        assert_eq!(parse_777(&b).doors, Doors::Open);
    }
}
