//! Sound packs: folders of recorded announcements, optionally tied to an
//! airline so the right pack plays when you fly for that airline.
//!
//! Layout: packs\<pack>\<script key>\*.ogg|wav|mp3|flac, plus pack.toml:
//!
//! ```toml
//! name = "Norwegian"
//! airlines = ["NAX"]   # empty: used for any airline without its own pack
//! play_all = false     # true plays every file in order (e.g. Norwegian, then English)
//! ```

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::announcements::SCRIPTS;
use crate::config::{packs_dir, sounds_dir};

const AUDIO: &[&str] = &["wav", "mp3", "ogg", "flac"];
const UNSORTED: &str = "_unsorted";
const MAX_DOWNLOAD: u64 = 1 << 30;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Manifest {
    pub name: String,
    pub airlines: Vec<String>,
    pub play_all: bool,
}

#[derive(Clone, Debug)]
pub struct Pack {
    /// Folder name.
    pub id: String,
    pub manifest: Manifest,
    /// Script keys that have at least one recording.
    pub covers: Vec<&'static str>,
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

pub fn list() -> Vec<Pack> {
    let Ok(entries) = std::fs::read_dir(packs_dir()) else { return Vec::new() };
    let mut packs: Vec<Pack> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let dir = e.path();
            let manifest = std::fs::read_to_string(dir.join("pack.toml"))
                .ok()
                .and_then(|s| toml::from_str(&s).ok())
                .unwrap_or_default();
            Pack {
                id: e.file_name().to_string_lossy().to_string(),
                manifest,
                covers: SCRIPTS.iter().filter(|s| !audio_files(&dir.join(s.key)).is_empty()).map(|s| s.key).collect(),
                unsorted: audio_files(&dir.join(UNSORTED)).len(),
            }
        })
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

/// Files to play for one announcement: from the pack first, then your own
/// sounds folder. Empty means use speech.
pub fn recordings(key: &str, pack: Option<&Pack>) -> Vec<PathBuf> {
    if let Some(p) = pack {
        let files = audio_files(&p.dir().join(key));
        if !files.is_empty() {
            return if p.manifest.play_all { files } else { vec![pick(&files)] };
        }
    }
    let files = audio_files(&sounds_dir().join(key));
    if files.is_empty() { Vec::new() } else { vec![pick(&files)] }
}

pub struct Versions {
    pub files: Vec<PathBuf>,
    /// "the Norwegian pack" or "your recordings folder".
    pub from: String,
}

/// Every recording that could play for an announcement, and where they're from.
pub fn versions(key: &str, pack: Option<&Pack>) -> Versions {
    if let Some(p) = pack {
        let files = audio_files(&p.dir().join(key));
        if !files.is_empty() {
            return Versions { files, from: format!("the {} pack", p.title()) };
        }
    }
    Versions { files: audio_files(&sounds_dir().join(key)), from: "your recordings folder".to_string() }
}

fn pick(files: &[PathBuf]) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    files[nanos as usize % files.len()].clone()
}

/// Lower-case words separated by single spaces, padded: " seat belt off ".
fn words(s: &str) -> String {
    let spaced: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { ' ' }).collect();
    format!(" {} ", spaced.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Phrases that identify each announcement in file and folder names. Order
/// matters: more specific entries come first ("disarm" before "arm").
const GUESSES: &[(&str, &[&str])] = &[
    ("disarm_doors", &["disarm", "doors manual", "doors to manual"]),
    ("arm_doors", &["arm doors", "doors arm", "arm door", "arming", "doors automatic", "doors to automatic", "cross check", "crosscheck"]),
    ("safety_demo", &["safety"]),
    ("go_around", &["go around", "goaround", "missed approach"]),
    ("turbulence", &["turbulence", "turb"]),
    ("captain_welcome", &["captain welcome", "welcome captain", "pilot welcome", "captain intro", "flight deck welcome", "captain departure", "captain boarding", "cpt welcome"]),
    ("captain_cruise", &["cruise", "captain update", "midflight", "mid flight", "inflight captain"]),
    ("captain_descent", &["descent", "top of descent", "tod"]),
    ("boarding_welcome", &["boarding", "welcome aboard", "welcome on board", "preboarding"]),
    ("crew_seats_takeoff", &["takeoff", "take off", "seats departure", "departure seats", "before departure"]),
    ("crew_seats_landing", &["seats landing", "landing seats", "crew landing", "before landing", "seats for landing", "final approach"]),
    ("prepare_cabin_landing", &["prepare", "approach", "landing prep", "cabin secure", "secure cabin"]),
    ("after_landing", &["after landing", "landed", "arrival", "arrived", "welcome to", "touchdown", "taxi in"]),
    ("disembark", &["disembark", "deboard", "deplan", "goodbye", "farewell", "at gate", "gate arrival"]),
    ("belts_off_climb", &["seatbelt off", "seat belt off", "seatbelts off", "belts off", "belt sign off", "after takeoff", "service", "duty free", "meal"]),
];

/// Which announcement a file is for, judged by its path inside the pack.
pub fn guess_key(rel: &Path) -> Option<&'static str> {
    // A folder (or file) named exactly like a script key wins.
    for part in rel.iter().rev() {
        let w = words(&Path::new(part).with_extension("").to_string_lossy()).trim().replace(' ', "_");
        if let Some(s) = SCRIPTS.iter().find(|s| s.key == w) {
            return Some(s.key);
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

#[derive(Debug)]
pub struct Report {
    pub id: String,
    pub sorted: usize,
    pub unsorted: usize,
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

/// Copies a folder of recordings in as a new pack, sorting each file into
/// its announcement. Files it can't place go to _unsorted for you to move.
pub fn install_folder(src: &Path, name: &str, airline: &str) -> Result<Report, String> {
    install_folder_in(&packs_dir(), src, name, airline)
}

fn install_folder_in(root: &Path, src: &Path, name: &str, airline: &str) -> Result<Report, String> {
    let mut files = Vec::new();
    walk(src, src, &mut files);
    if files.is_empty() {
        return Err("no .wav, .mp3, .ogg or .flac files in it".into());
    }
    let mut manifest = find_manifest(src).unwrap_or_default();
    if manifest.name.is_empty() {
        manifest.name = name.to_string();
    }
    let airline = airline.trim().to_ascii_uppercase();
    if !airline.is_empty() && !manifest.airlines.iter().any(|a| a.eq_ignore_ascii_case(&airline)) {
        manifest.airlines.push(airline);
    }
    let base = safe_id(&manifest.name);
    let id = (1..)
        .map(|n| if n == 1 { base.clone() } else { format!("{base} {n}") })
        .find(|id| !root.join(id).exists())
        .unwrap();
    let dest = root.join(&id);
    let (mut sorted, mut unsorted) = (0, 0);
    for rel in &files {
        let key = guess_key(rel);
        let folder = dest.join(key.unwrap_or(UNSORTED));
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let file_name = rel.file_name().unwrap_or_default().to_string_lossy().to_string();
        let mut target = folder.join(&file_name);
        let mut n = 2;
        while target.exists() {
            target = folder.join(format!("{n}_{file_name}"));
            n += 1;
        }
        std::fs::copy(src.join(rel), &target).map_err(|e| format!("{}: {e}", rel.display()))?;
        if key.is_some() { sorted += 1 } else { unsorted += 1 }
    }
    let text = toml::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(dest.join("pack.toml"), text).map_err(|e| e.to_string())?;
    Ok(Report { id, sorted, unsorted })
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("cabin-announcer-{tag}-{nanos}"))
}

pub fn install_zip(zip_path: &Path, name: &str, airline: &str) -> Result<Report, String> {
    install_zip_in(&packs_dir(), zip_path, name, airline)
}

fn install_zip_in(root: &Path, zip_path: &Path, name: &str, airline: &str) -> Result<Report, String> {
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
        install_folder_in(root, &tmp, name, airline)
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
        assert_eq!(g("DoorsDisarm.ogg"), None);
        assert_eq!(g("Doors Disarm.ogg"), Some("disarm_doors"));
        assert_eq!(g("cabin/arm doors and cross-check.mp3"), Some("arm_doors"));
        assert_eq!(g("Captain Welcome - Norwegian.wav"), Some("captain_welcome"));
        assert_eq!(g("09_Turbulence.ogg"), Some("turbulence"));
        assert_eq!(g("cabin_crew_prepare_for_takeoff.ogg"), Some("crew_seats_takeoff"));
        assert_eq!(g("landing/after_landing_no.ogg"), Some("after_landing"));
        assert_eq!(g("random music.ogg"), None);
    }

    #[test]
    fn installs_a_zip_and_sorts_it() {
        use std::io::Write;
        let root = temp_dir("test-root");
        let zip_path = temp_dir("test-zip").with_extension("zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            for name in ["Nor Pack/Safety Demo.wav", "Nor Pack/boarding/1.ogg", "Nor Pack/boarding/2.ogg", "Nor Pack/music.mp3", "Nor Pack/readme.txt", "../escape.wav"] {
                w.start_file(name, opts).unwrap();
                w.write_all(b"RIFF").unwrap();
            }
            w.finish().unwrap();
        }
        let r = install_zip_in(&root, &zip_path, "Nor Pack", "nax").unwrap();
        assert_eq!((r.sorted, r.unsorted), (3, 1));
        let dir = root.join(&r.id);
        assert!(dir.join("safety_demo/Safety Demo.wav").is_file());
        assert_eq!(audio_files(&dir.join("boarding_welcome")).len(), 2);
        assert!(dir.join("_unsorted/music.mp3").is_file());
        assert!(!dir.join("readme.txt").exists());
        assert!(!root.join("escape.wav").exists() && !root.parent().unwrap().join("escape.wav").exists());
        let m: Manifest = toml::from_str(&std::fs::read_to_string(dir.join("pack.toml")).unwrap()).unwrap();
        assert_eq!(m.airlines, vec!["NAX"]);
        // Installing again keeps the first copy.
        let again = install_zip_in(&root, &zip_path, "Nor Pack", "").unwrap();
        assert_eq!(again.id, "Nor Pack 2");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&zip_path);
    }

    #[test]
    fn manifests_round_trip() {
        let m = Manifest { name: "Norwegian".into(), airlines: vec!["NAX".into()], play_all: true };
        let back: Manifest = toml::from_str(&toml::to_string_pretty(&m).unwrap()).unwrap();
        assert_eq!(back, m);
        let p = Pack { id: "n".into(), manifest: m, covers: vec![], unsorted: 0 };
        assert!(p.serves("NAX") && p.serves("NSZ") && !p.serves("BAW"));
    }
}
