//! Starting with MSFS: an entry in the sim's exe.xml launches the app with
//! `--autostart`, and the app closes itself when the sim quits.

use std::path::{Path, PathBuf};

pub const FLAG: &str = "--autostart";
const NAME: &str = "<Name>Cabin Announcer</Name>";

fn exe_xml_files() -> Vec<PathBuf> {
    crate::msfs::config_dirs().into_iter().map(|d| d.join("exe.xml")).collect()
}

/// exe.xml is Windows-1252. Mapping bytes to chars one-to-one keeps any
/// non-ASCII bytes exactly as they were when the file is written back.
fn read(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| b.into_iter().map(char::from).collect())
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    let bytes: Option<Vec<u8>> = text.chars().map(|c| u8::try_from(c as u32).ok()).collect();
    let bytes = bytes.ok_or("the app's path has characters exe.xml can't hold")?;
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Removes our <Launch.Addon> block, if there is one.
fn without_entry(xml: &str) -> String {
    let Some(name_at) = xml.find(NAME) else { return xml.to_string() };
    let (Some(start), Some(end)) = (xml[..name_at].rfind("<Launch.Addon>"), xml[name_at..].find("</Launch.Addon>")) else {
        return xml.to_string();
    };
    let end = name_at + end + "</Launch.Addon>".len();
    // Take the indentation before and the line break after with it.
    let start = xml[..start].rfind('\n').map_or(start, |n| n + 1);
    let end = if xml[end..].starts_with("\r\n") { end + 2 } else if xml[end..].starts_with('\n') { end + 1 } else { end };
    format!("{}{}", &xml[..start], &xml[end..])
}

fn with_entry(xml: &str, exe: &Path) -> Result<String, String> {
    let entry = format!(
        "  <Launch.Addon>\r\n    {NAME}\r\n    <Disabled>False</Disabled>\r\n    <Path>{}</Path>\r\n    <CommandLine>{FLAG}</CommandLine>\r\n  </Launch.Addon>\r\n",
        escape(&exe.display().to_string())
    );
    let clean = without_entry(xml);
    let close = clean.rfind("</SimBase.Document>").ok_or("exe.xml doesn't look like an MSFS launch file")?;
    Ok(format!("{}{entry}{}", &clean[..close], &clean[close..]))
}

const EMPTY: &str = "<?xml version=\"1.0\" encoding=\"windows-1252\"?>\r\n<SimBase.Document Type=\"Launch\" version=\"1,0\">\r\n  <Descr>Launch</Descr>\r\n  <Filename>EXE.xml</Filename>\r\n  <Disabled>False</Disabled>\r\n  <Launch.ManualLoad>False</Launch.ManualLoad>\r\n</SimBase.Document>\r\n";

pub fn is_enabled() -> bool {
    exe_xml_files().iter().any(|f| read(f).is_some_and(|x| x.contains(NAME)))
}

/// Adds or removes the launch entry in every MSFS 2024 install found. The
/// first change keeps a backup as exe.xml.cabin-announcer.bak.
pub fn set_enabled(on: bool) -> Result<usize, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let files = exe_xml_files();
    if files.is_empty() {
        return Err("couldn't find MSFS 2024's settings folder".into());
    }
    for f in &files {
        let old = read(f);
        let new = match (&old, on) {
            (Some(x), true) => with_entry(x, &exe)?,
            (Some(x), false) => without_entry(x),
            (None, true) => with_entry(EMPTY, &exe)?,
            (None, false) => continue,
        };
        if old.as_deref() != Some(new.as_str()) {
            let backup = f.with_extension("xml.cabin-announcer.bak");
            if let Some(x) = &old {
                if !backup.exists() {
                    write(&backup, x)?;
                }
            }
            write(f, &new)?;
        }
    }
    Ok(files.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "<?xml version=\"1.0\" encoding=\"windows-1252\"?>\r\n<SimBase.Document Type=\"Launch\" version=\"1,0\">\r\n  <Launch.Addon>\r\n    <Name>Couatl</Name>\r\n    <Path>C:\\couatl64_boot.exe</Path>\r\n  </Launch.Addon>\r\n</SimBase.Document>\r\n";

    #[test]
    fn adds_and_removes_only_our_entry() {
        let exe = Path::new(r"G:\Apps\Cabin Announcer & Co\cabin-announcer.exe");
        let on = with_entry(SAMPLE, exe).unwrap();
        assert!(on.contains("<Name>Couatl</Name>"));
        assert!(on.contains(r"<Path>G:\Apps\Cabin Announcer &amp; Co\cabin-announcer.exe</Path>"));
        assert!(on.trim_end().ends_with("</SimBase.Document>"));
        // Adding twice leaves one entry.
        assert_eq!(with_entry(&on, exe).unwrap().matches(NAME).count(), 1);
        assert_eq!(without_entry(&on), SAMPLE);
    }

    #[test]
    fn keeps_windows_1252_bytes() {
        let dir = std::env::temp_dir().join(format!("ca-exe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("exe.xml");
        let mut bytes = SAMPLE.as_bytes().to_vec();
        bytes.splice(0..0, [0xE9u8]);
        std::fs::write(&f, &bytes).unwrap();
        let text = read(&f).unwrap();
        write(&f, &text).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
