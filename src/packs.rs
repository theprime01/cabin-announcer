//! Sound packs: folders of recorded announcements, optionally tied to an
//! airline so the right pack plays when you fly for that airline.
//!
//! Layout: packs\<pack>\<script key>\*.ogg|wav|mp3|flac, plus pack.toml:
//!
//! ```toml
//! name = "Norwegian"
//! airlines = ["NAX"]   # empty: used for any airline without its own pack
//! play_all = false     # true plays every file in order (e.g. Norwegian, then English)
//! chime = false        # true adds the app's PA chime (for recordings without one)
//! ```
//!
//! File names can carry Fenix-style tags: `BoardingWelcome[Morning].ogg` only
//! plays in the morning, `ArmDoors[2].ogg` is one of several versions.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::announcements::{BOARDING_MUSIC, CABIN_AMBIENCE, SCRIPTS};
use crate::config::packs_dir;

const AUDIO: &[&str] = &["wav", "mp3", "ogg", "flac"];
const UNSORTED: &str = "_unsorted";
const MAX_DOWNLOAD: u64 = 1 << 30;
const DAYPARTS: &[&str] = &["morning", "afternoon", "evening", "night"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Manifest {
    pub name: String,
    pub airlines: Vec<String>,
    pub play_all: bool,
    pub chime: bool,
}

#[derive(Clone, Debug)]
pub struct Pack {
    /// Folder name.
    pub id: String,
    pub manifest: Manifest,
    /// Script keys that have at least one recording.
    pub covers: Vec<&'static str>,
    pub music: usize,
    pub ambience: usize,
    pub unsorted: usize,
}

impl Pack {
    pub fn dir(&self) -> PathBuf {
        packs_dir().join(&self.id)
    }

    pub fn title(&self) -> &str {
        if self.manifest.name.is_empty() { &self.id } else { &self.manifest.name }
    }

    /// True when the pack is for this airline (sister codes count).
    pub fn serves(&self, icao: &str) -> bool {
        let primary = |c: &str| crate::airline::by_code(c).map_or(c.to_ascii_uppercase(), |a| a.icao.to_string());
        self.manifest.airlines.iter().any(|a| primary(a) == primary(icao))
    }
}

fn is_audio(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| AUDIO.contains(&e.to_ascii_lowercase().as_str()))
}

fn audio_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_audio(p)).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn read_pack(dir: PathBuf, id: String) -> Pack {
    let manifest = std::fs::read_to_string(dir.join("pack.toml")).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default();
    Pack {
        id,
        manifest,
        covers: SCRIPTS.iter().filter(|s| !audio_files(&dir.join(s.key)).is_empty()).map(|s| s.key).collect(),
        music: audio_files(&dir.join(BOARDING_MUSIC)).len(),
        ambience: audio_files(&dir.join(CABIN_AMBIENCE)).len(),
        unsorted: audio_files(&dir.join(UNSORTED)).len(),
    }
}

pub fn list() -> Vec<Pack> {
    let Ok(entries) = std::fs::read_dir(packs_dir()) else { return Vec::new() };
    let mut packs: Vec<Pack> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| read_pack(e.path(), e.file_name().to_string_lossy().to_string()))
        .collect();
    packs.sort_by(|a, b| a.title().to_lowercase().cmp(&b.title().to_lowercase()));
    packs
}

/// The pack to use: the chosen one, or (automatic) the detected airline's,
/// else a pack that isn't tied to any airline.
pub fn active(choice: &str, airline_icao: Option<&str>) -> Option<Pack> {
    if choice == "none" {
        return None;
    }
    let packs = list();
    if !choice.is_empty() {
        return packs.into_iter().find(|p| p.id == choice);
    }
    if let Some(icao) = airline_icao {
        if let Some(p) = packs.iter().find(|p| p.serves(icao)) {
            return Some(p.clone());
        }
    }
    packs.into_iter().find(|p| p.manifest.airlines.is_empty())
}

/// `BoardingWelcome[Morning][2].ogg` -> ("BoardingWelcome", ["Morning", "2"]).
fn split_tags(stem: &str) -> (String, Vec<String>) {
    let base = stem.split('[').next().unwrap_or(stem).trim().to_string();
    let tags = stem.split('[').skip(1).filter_map(|t| t.split(']').next()).map(|t| t.trim().to_string()).collect();
    (base, tags)
}

/// The time of day a recording is for, if it says.
fn daypart_of(path: &Path) -> Option<&'static str> {
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let (_, tags) = split_tags(&stem);
    let w = words(&stem);
    DAYPARTS
        .iter()
        .copied()
        .find(|d| tags.iter().any(|t| t.eq_ignore_ascii_case(d)) || w.contains(&format!(" {d} ")))
}

/// Recordings suited to the time of day: ones for this daypart (night falls
/// back to evening), else ones for any time, else whatever there is.
fn for_time(files: &[PathBuf], daypart: Option<&str>) -> Vec<PathBuf> {
    let tagged = |d: &str| files.iter().filter(|f| daypart_of(f) == Some(d)).cloned().collect::<Vec<_>>();
    if let Some(d) = daypart {
        let now = tagged(d);
        if !now.is_empty() {
            return now;
        }
        if d == "night" {
            let evening = tagged("evening");
            if !evening.is_empty() {
                return evening;
            }
        }
    }
    let untimed: Vec<PathBuf> = files.iter().filter(|f| daypart_of(f).is_none()).cloned().collect();
    if untimed.is_empty() { files.to_vec() } else { untimed }
}

/// Files to play for one announcement. Empty means the pack doesn't have it.
pub fn recordings(key: &str, pack: Option<&Pack>, daypart: Option<&str>) -> Vec<PathBuf> {
    let Some(p) = pack else { return Vec::new() };
    let files = for_time(&audio_files(&p.dir().join(key)), daypart);
    if files.is_empty() {
        Vec::new()
    } else if p.manifest.play_all {
        files
    } else {
        vec![pick(&files)]
    }
}

/// Every recording a pack has for an announcement, for previewing.
pub fn versions(key: &str, pack: Option<&Pack>) -> Vec<PathBuf> {
    pack.map(|p| audio_files(&p.dir().join(key))).unwrap_or_default()
}

/// Boarding music or cabin ambience tracks.
pub fn background(folder: &str, pack: Option<&Pack>) -> Vec<PathBuf> {
    versions(folder, pack)
}

pub fn pick(files: &[PathBuf]) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    files[nanos as usize % files.len()].clone()
}

/// Lower-case words separated by single spaces, padded: " seat belt off ".
/// CamelCase is split too: "DoorsDisarm" -> " doors disarm ".
fn words(s: &str) -> String {
    let mut spaced = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_ascii_uppercase() && prev_lower {
            spaced.push(' ');
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        spaced.push(if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { ' ' });
    }
    format!(" {} ", spaced.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Fenix A320 announcement names (also used by Universal Announcer packs).
const FENIX: &[(&str, &str)] = &[
    ("AfterLanding", "after_landing"),
    ("AfterTakeoff", "belts_off_climb"),
    ("ArmDoors", "arm_doors"),
    ("BoardingComplete", "boarding_complete"),
    ("BoardingMusic", BOARDING_MUSIC),
    ("BoardingWelcome", "boarding_welcome"),
    ("CabinDim", "cabin_dim_landing"),
    ("CabinDimTakeoff", "cabin_dim_takeoff"),
    ("CallCabinSecureLanding", "cabin_secure_landing"),
    ("CallCabinSecureTakeoff", "cabin_secure_takeoff"),
    ("CrewSeatsLanding", "crew_seats_landing"),
    ("CrewSeatsTakeoff", "crew_seats_takeoff"),
    ("DescentSeatbelts", "prepare_cabin_landing"),
    ("DisarmDoors", "disarm_doors"),
    ("DisembarkStarted", "disembark"),
    ("FastenSeatbelt", "turbulence"),
    ("PreSafetyBriefing", "pre_safety_briefing"),
    ("SafetyBriefing", "safety_demo"),
];

/// Fenix variants that don't fit a normal flight, left out on import.
const SKIPPED_TAGS: &[&str] = &["refueling", "refuelling"];

/// Phrases that identify each announcement in other file and folder names.
/// Order matters: more specific entries come first ("disarm" before "arm").
const GUESSES: &[(&str, &[&str])] = &[
    (BOARDING_MUSIC, &["boarding music", "music"]),
    (CABIN_AMBIENCE, &["ambience", "ambient", "cabin noise", "cabin sound"]),
    ("disarm_doors", &["disarm", "doors manual", "doors to manual"]),
    ("arm_doors", &["arm doors", "doors arm", "arm door", "arming", "doors automatic", "doors to automatic", "cross check", "crosscheck"]),
    ("pre_safety_briefing", &["pre safety", "presafety", "before safety"]),
    ("safety_demo", &["safety"]),
    ("go_around", &["go around", "goaround", "missed approach"]),
    ("turbulence", &["turbulence", "turb", "fasten seatbelt", "fasten seat belt", "fasten seatbelts"]),
    ("takeoff_delay", &["takeoff delay", "take off delay", "runway delay", "slot", "holding point"]),
    ("holding", &["holding", "hold"]),
    ("departure_delay", &["departure delay", "delay", "late"]),
    ("cabin_dim_takeoff", &["cabin dim takeoff", "dim takeoff", "lights takeoff"]),
    ("cabin_dim_landing", &["cabin dim", "dim landing", "lights landing"]),
    ("cabin_secure_takeoff", &["secure takeoff", "cabin secure takeoff", "prepare for takeoff", "prepare takeoff"]),
    ("cabin_secure_landing", &["secure landing", "cabin secure landing", "prepare for landing", "prepare landing"]),
    ("prepare_cabin_landing", &["descent seatbelt", "descent seatbelts", "descent seat belts"]),
    ("captain_welcome", &["captain welcome", "welcome captain", "pilot welcome", "captain intro", "flight deck welcome", "captain departure", "captain boarding", "cpt welcome"]),
    ("captain_cruise", &["cruise", "captain update", "midflight", "mid flight", "inflight captain"]),
    ("captain_descent", &["descent", "top of descent", "tod"]),
    ("boarding_complete", &["boarding complete", "boarding completed", "boarding finished"]),
    ("boarding_welcome", &["boarding", "welcome aboard", "welcome on board", "preboarding"]),
    ("crew_seats_takeoff", &["takeoff", "take off", "seats departure", "departure seats", "before departure"]),
    ("crew_seats_landing", &["seats landing", "landing seats", "crew landing", "before landing", "seats for landing", "final approach"]),
    ("prepare_cabin_landing", &["prepare", "approach", "landing prep", "cabin secure", "secure cabin"]),
    ("after_landing", &["after landing", "landed", "arrival", "arrived", "welcome to", "touchdown", "taxi in"]),
    ("disembark", &["disembark", "deboard", "deplan", "goodbye", "farewell", "at gate", "gate arrival"]),
    ("belts_off_climb", &["seatbelt off", "seat belt off", "seatbelts off", "belts off", "belt sign off", "after takeoff", "service", "duty free", "meal"]),
];

fn known_folder(name: &str) -> Option<&'static str> {
    let w = words(name).trim().replace(' ', "_");
    SCRIPTS.iter().map(|s| s.key).chain([BOARDING_MUSIC, CABIN_AMBIENCE]).find(|k| *k == w)
}

/// Which announcement a file is for, judged by its path inside the pack.
pub fn guess_key(rel: &Path) -> Option<&'static str> {
    // An exact Fenix name, then a folder (or file) named like a script key.
    let stem = rel.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let (base, _) = split_tags(&stem);
    if let Some((_, key)) = FENIX.iter().find(|(f, _)| f.eq_ignore_ascii_case(&base)) {
        return Some(key);
    }
    for part in rel.iter().rev() {
        if let Some(k) = known_folder(&Path::new(part).with_extension("").to_string_lossy()) {
            return Some(k);
        }
    }
    // Then the file name, then its folders.
    for part in rel.iter().rev() {
        let w = words(&Path::new(part).with_extension("").to_string_lossy());
        if let Some((key, _)) = GUESSES.iter().find(|(_, phrases)| phrases.iter().any(|p| w.contains(&format!(" {p} ")))) {
            return Some(key);
        }
    }
    None
}

/// The name a file gets in the pack. A time of day given by a folder
/// ("morning\welcome.ogg") moves into the name as a tag.
fn stored_name(rel: &Path) -> String {
    let name = rel.file_name().unwrap_or_default().to_string_lossy().to_string();
    if daypart_of(rel).is_some() {
        return name;
    }
    let folder_time = rel
        .parent()
        .into_iter()
        .flat_map(|p| p.iter())
        .find_map(|part| DAYPARTS.iter().find(|d| part.to_string_lossy().eq_ignore_ascii_case(d)));
    match (folder_time, rel.file_stem(), rel.extension()) {
        (Some(d), Some(stem), Some(ext)) => {
            let mut t = d.to_string();
            t[..1].make_ascii_uppercase();
            format!("{}[{t}].{}", stem.to_string_lossy(), ext.to_string_lossy())
        }
        _ => name,
    }
}

#[derive(Debug)]
pub struct Report {
    pub id: String,
    pub sorted: usize,
    pub unsorted: usize,
    pub skipped: usize,
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if is_audio(&p) {
            out.push(p.strip_prefix(base).unwrap_or(&p).to_path_buf());
        }
    }
}

fn find_manifest(dir: &Path) -> Option<Manifest> {
    if let Ok(s) = std::fs::read_to_string(dir.join("pack.toml")) {
        return toml::from_str(&s).ok();
    }
    std::fs::read_dir(dir).ok()?.flatten().filter(|e| e.path().is_dir()).find_map(|e| find_manifest(&e.path()))
}

fn safe_id(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || " -_()".contains(c) { c } else { '_' }).collect();
    let s = s.trim().to_string();
    if s.is_empty() { "pack".into() } else { s }
}

struct Install<'a> {
    name: &'a str,
    airline: &'a str,
    /// Replace a pack with the same name instead of adding "2".
    replace: bool,
}

/// Copies a folder of recordings in as a new pack, sorting each file into
/// its announcement. Files it can't place go to _unsorted for you to move.
pub fn install_folder(src: &Path, name: &str, airline: &str) -> Result<Report, String> {
    install_folder_in(&packs_dir(), src, &Install { name, airline, replace: false })
}

fn install_folder_in(root: &Path, src: &Path, how: &Install) -> Result<Report, String> {
    let mut files = Vec::new();
    walk(src, src, &mut files);
    if files.is_empty() {
        return Err("no .wav, .mp3, .ogg or .flac files in it".into());
    }
    let mut manifest = find_manifest(src).unwrap_or_default();
    if manifest.name.is_empty() {
        manifest.name = how.name.to_string();
    }
    let airline = how.airline.trim().to_ascii_uppercase();
    if !airline.is_empty() && !manifest.airlines.iter().any(|a| a.eq_ignore_ascii_case(&airline)) {
        manifest.airlines.push(airline);
    }
    let base = safe_id(&manifest.name);
    let id = if how.replace {
        let _ = std::fs::remove_dir_all(root.join(&base));
        base
    } else {
        (1..).map(|n| if n == 1 { base.clone() } else { format!("{base} {n}") }).find(|id| !root.join(id).exists()).unwrap()
    };
    let dest = root.join(&id);
    let (mut sorted, mut unsorted, mut skipped) = (0, 0, 0);
    for rel in &files {
        let stem = rel.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if split_tags(&stem).1.iter().any(|t| SKIPPED_TAGS.contains(&t.to_ascii_lowercase().as_str())) {
            skipped += 1;
            continue;
        }
        let key = guess_key(rel);
        let folder = dest.join(key.unwrap_or(UNSORTED));
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let file_name = stored_name(rel);
        let mut target = folder.join(&file_name);
        let mut n = 2;
        while target.exists() {
            target = folder.join(format!("{n}_{file_name}"));
            n += 1;
        }
        std::fs::copy(src.join(rel), &target).map_err(|e| format!("{}: {e}", rel.display()))?;
        if key.is_some() { sorted += 1 } else { unsorted += 1 }
    }
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let text = toml::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(dest.join("pack.toml"), text).map_err(|e| e.to_string())?;
    Ok(Report { id, sorted, unsorted, skipped })
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("cabin-announcer-{tag}-{nanos}"))
}

pub fn install_zip(zip_path: &Path, name: &str, airline: &str) -> Result<Report, String> {
    install_zip_in(&packs_dir(), zip_path, &Install { name, airline, replace: false })
}

fn install_zip_in(root: &Path, zip_path: &Path, how: &Install) -> Result<Report, String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("not a zip file: {e}"))?;
    let tmp = temp_dir("unzip");
    let result = (|| {
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
            // enclosed_name rejects absolute paths and "..", so nothing lands outside tmp.
            let Some(rel) = f.enclosed_name() else { continue };
            let keep = rel.file_name().is_some_and(|n| n == "pack.toml") || is_audio(&rel);
            if f.is_dir() || !keep {
                continue;
            }
            let out = tmp.join(rel);
            std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
            let mut w = std::fs::File::create(&out).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, &mut w).map_err(|e| e.to_string())?;
        }
        install_folder_in(root, &tmp, how)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Downloads a zip from a link you trust (e.g. a pack you've bought or been
/// given) and installs it.
pub fn install_url(url: &str, airline: &str) -> Result<Report, String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("the link must start with https://".into());
    }
    let resp = ureq::get(url).call().map_err(|e| format!("download failed: {e}"))?;
    let tmp = temp_dir("download");
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let zip_path = tmp.join("pack.zip");
    let result = (|| {
        let mut out = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
        let copied = std::io::copy(&mut resp.into_reader().take(MAX_DOWNLOAD + 1), &mut out).map_err(|e| e.to_string())?;
        if copied > MAX_DOWNLOAD {
            return Err("that download is over 1 GB".to_string());
        }
        drop(out);
        let name = url.rsplit('/').next().unwrap_or("pack").split(['?', '#']).next().unwrap_or("pack");
        let name = Path::new(name).file_stem().map_or("pack".into(), |s| s.to_string_lossy().to_string());
        install_zip(&zip_path, &name, airline)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Installs whatever was dropped or typed: a zip, a folder, or a link.
pub fn install(source: &str, airline: &str) -> Result<Report, String> {
    let source = source.trim().trim_matches('"');
    if source.starts_with("http://") || source.starts_with("https://") {
        return install_url(source, airline);
    }
    let p = Path::new(source);
    let name = p.file_stem().map_or("pack".into(), |s| s.to_string_lossy().to_string());
    if p.is_dir() {
        install_folder(p, &name, airline)
    } else if p.is_file() {
        install_zip(p, &name, airline)
    } else {
        Err(format!("can't find {source}"))
    }
}

/// Fenix-format announcement sets in the Community folder: any
/// `<package>\Announcements\<set>\` with recordings in it. The set's folder
/// name is the airline code ("BAW"), or "Default" for every airline.
pub fn find_fenix_sets() -> Vec<(String, PathBuf)> {
    let mut sets = Vec::new();
    for community in crate::msfs::community_dirs() {
        for pkg in std::fs::read_dir(&community).into_iter().flatten().flatten() {
            let ann = pkg.path().join("Announcements");
            for set in std::fs::read_dir(&ann).into_iter().flatten().flatten() {
                let dir = set.path();
                if dir.is_dir() && !audio_files(&dir).is_empty() {
                    sets.push((set.file_name().to_string_lossy().to_string(), dir));
                }
            }
        }
    }
    sets.sort();
    sets
}

/// Imports every Fenix set as its own pack ("Fenix BAW", "Fenix Default"),
/// replacing earlier imports so re-running picks up changes.
pub fn import_fenix() -> Result<Vec<Report>, String> {
    import_fenix_into(&packs_dir(), &find_fenix_sets())
}

fn import_fenix_into(root: &Path, sets: &[(String, PathBuf)]) -> Result<Vec<Report>, String> {
    if sets.is_empty() {
        return Err("no Fenix announcements found in your Community folder".into());
    }
    let mut reports = Vec::new();
    for (set, dir) in sets {
        let code = if set.len() == 3 && set.chars().all(|c| c.is_ascii_alphabetic()) { set.to_ascii_uppercase() } else { String::new() };
        let name = format!("Fenix {set}");
        reports.push(install_folder_in(root, dir, &Install { name: &name, airline: &code, replace: true })?);
    }
    Ok(reports)
}

pub fn save_manifest(id: &str, m: &Manifest) -> Result<(), String> {
    let dir = packs_dir().join(id);
    if !dir.is_dir() {
        return Err("no such pack".into());
    }
    let text = toml::to_string_pretty(m).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("pack.toml"), text).map_err(|e| e.to_string())
}

pub fn remove(id: &str) -> Result<(), String> {
    let dir = packs_dir().join(id);
    if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") || !dir.is_dir() {
        return Err("no such pack".into());
    }
    std::fs::remove_dir_all(dir).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_keys_from_names() {
        let g = |s: &str| guess_key(Path::new(s));
        assert_eq!(g("Fenix/Cabin/Safety Demo EN.ogg"), Some("safety_demo"));
        assert_eq!(g("pack/disarm_doors/1.ogg"), Some("disarm_doors"));
        assert_eq!(g("DoorsDisarm.ogg"), Some("disarm_doors"));
        assert_eq!(g("cabin/arm doors and cross-check.mp3"), Some("arm_doors"));
        assert_eq!(g("Captain Welcome - Norwegian.wav"), Some("captain_welcome"));
        assert_eq!(g("09_Turbulence.ogg"), Some("turbulence"));
        assert_eq!(g("cabin_crew_prepare_for_takeoff.ogg"), Some("cabin_secure_takeoff"));
        assert_eq!(g("landing/after_landing_no.ogg"), Some("after_landing"));
        assert_eq!(g("Runway delay.mp3"), Some("takeoff_delay"));
        assert_eq!(g("Holding pattern.mp3"), Some("holding"));
        assert_eq!(g("Lounge Music 2.mp3"), Some(BOARDING_MUSIC));
        assert_eq!(g("random tune.ogg"), None);
    }

    #[test]
    fn fenix_names_map_exactly() {
        let g = |s: &str| guess_key(Path::new(s));
        assert_eq!(g("BAW/BoardingWelcome[Morning].ogg"), Some("boarding_welcome"));
        assert_eq!(g("BAW/PreSafetyBriefing[1][A320].ogg"), Some("pre_safety_briefing"));
        assert_eq!(g("BAW/SafetyBriefing.ogg"), Some("safety_demo"));
        assert_eq!(g("Default/CabinDim.ogg"), Some("cabin_dim_landing"));
        assert_eq!(g("Default/CabinDimTakeoff.ogg"), Some("cabin_dim_takeoff"));
        assert_eq!(g("Default/DescentSeatbelts.ogg"), Some("prepare_cabin_landing"));
        assert_eq!(g("Default/FastenSeatbelt.ogg"), Some("turbulence"));
        assert_eq!(g("EZY/BoardingMusic.ogg"), Some(BOARDING_MUSIC));
        for (name, key) in FENIX {
            assert_eq!(g(&format!("{name}.ogg")), Some(*key), "{name}");
        }
    }

    #[test]
    fn picks_by_time_of_day() {
        let files: Vec<PathBuf> = ["W[Morning].ogg", "W[Afternoon].ogg", "W[Evening].ogg"].iter().map(PathBuf::from).collect();
        assert_eq!(for_time(&files, Some("morning")), vec![PathBuf::from("W[Morning].ogg")]);
        assert_eq!(for_time(&files, Some("night")), vec![PathBuf::from("W[Evening].ogg")]);
        let mixed: Vec<PathBuf> = ["W[Morning].ogg", "W.ogg"].iter().map(PathBuf::from).collect();
        assert_eq!(for_time(&mixed, Some("evening")), vec![PathBuf::from("W.ogg")]);
        assert_eq!(for_time(&mixed, None), vec![PathBuf::from("W.ogg")]);
        let numbered: Vec<PathBuf> = ["A[1].ogg", "A[2].ogg"].iter().map(PathBuf::from).collect();
        assert_eq!(for_time(&numbered, Some("morning")).len(), 2);
        assert_eq!(stored_name(Path::new("evening/welcome.ogg")), "welcome[Evening].ogg");
        assert_eq!(stored_name(Path::new("x/W[Morning].ogg")), "W[Morning].ogg");
    }

    fn write(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"RIFF").unwrap();
    }

    #[test]
    fn installs_a_zip_and_sorts_it() {
        use std::io::Write;
        let root = temp_dir("test-root");
        let zip_path = temp_dir("test-zip").with_extension("zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            for name in ["Nor Pack/Safety Demo.wav", "Nor Pack/boarding/1.ogg", "Nor Pack/boarding/2.ogg", "Nor Pack/random tune.mp3", "Nor Pack/readme.txt", "../escape.wav"] {
                w.start_file(name, opts).unwrap();
                w.write_all(b"RIFF").unwrap();
            }
            w.finish().unwrap();
        }
        let how = Install { name: "Nor Pack", airline: "nax", replace: false };
        let r = install_zip_in(&root, &zip_path, &how).unwrap();
        assert_eq!((r.sorted, r.unsorted), (3, 1));
        let dir = root.join(&r.id);
        assert!(dir.join("safety_demo/Safety Demo.wav").is_file());
        assert_eq!(audio_files(&dir.join("boarding_welcome")).len(), 2);
        assert!(dir.join("_unsorted/random tune.mp3").is_file());
        assert!(!dir.join("readme.txt").exists());
        assert!(!root.join("escape.wav").exists() && !root.parent().unwrap().join("escape.wav").exists());
        let m: Manifest = toml::from_str(&std::fs::read_to_string(dir.join("pack.toml")).unwrap()).unwrap();
        assert_eq!(m.airlines, vec!["NAX"]);
        // Installing again keeps the first copy.
        let again = install_zip_in(&root, &zip_path, &Install { airline: "", ..how }).unwrap();
        assert_eq!(again.id, "Nor Pack 2");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&zip_path);
    }

    #[test]
    fn imports_fenix_sets() {
        let src = temp_dir("fenix-src");
        for f in ["BAW/BoardingWelcome[Morning].ogg", "BAW/BoardingWelcome[Refueling].ogg", "BAW/ArmDoors[2].ogg", "Default/SafetyBriefing[A320].ogg"] {
            write(&src.join(f));
        }
        let root = temp_dir("fenix-root");
        let sets = vec![("BAW".to_string(), src.join("BAW")), ("Default".to_string(), src.join("Default"))];
        let reports = import_fenix_into(&root, &sets).unwrap();
        assert_eq!(reports[0].id, "Fenix BAW");
        assert_eq!((reports[0].sorted, reports[0].skipped), (2, 1));
        let baw = read_pack(root.join("Fenix BAW"), "Fenix BAW".into());
        assert_eq!(baw.manifest.airlines, vec!["BAW"]);
        assert!(baw.covers.contains(&"boarding_welcome") && baw.covers.contains(&"arm_doors"));
        let default = read_pack(root.join("Fenix Default"), "Fenix Default".into());
        assert!(default.manifest.airlines.is_empty());
        // Re-importing replaces rather than duplicating.
        import_fenix_into(&root, &sets).unwrap();
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&src);
    }

    /// Imports the Fenix sets on this PC into the real packs folder and
    /// checks every recording decodes. Run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_fenix_import() {
        for r in import_fenix().unwrap() {
            let pack = read_pack(packs_dir().join(&r.id), r.id.clone());
            println!("{}: {} sorted, {} unsorted, {} skipped; covers {:?}; music {}", r.id, r.sorted, r.unsorted, r.skipped, pack.covers, pack.music);
            let mut files = Vec::new();
            walk(&pack.dir(), &pack.dir(), &mut files);
            for f in files {
                let path = pack.dir().join(&f);
                let dec = rodio::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
                assert!(dec.is_ok(), "{} doesn't decode", path.display());
            }
        }
    }

    #[test]
    fn manifests_round_trip() {
        let m = Manifest { name: "Norwegian".into(), airlines: vec!["NAX".into()], play_all: true, chime: true };
        let back: Manifest = toml::from_str(&toml::to_string_pretty(&m).unwrap()).unwrap();
        assert_eq!(back, m);
        let p = Pack { id: "n".into(), manifest: m, covers: vec![], music: 0, ambience: 0, unsorted: 0 };
        assert!(p.serves("NAX") && p.serves("NSZ") && !p.serves("BAW"));
    }
}
