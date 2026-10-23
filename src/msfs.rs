//! Where MSFS 2024 keeps its files, for Steam and Microsoft Store installs.

use std::path::PathBuf;

/// The folders holding UserCfg.opt and exe.xml.
pub fn config_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(a) = std::env::var_os("APPDATA") {
        v.push(PathBuf::from(a).join("Microsoft Flight Simulator 2024"));
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(l).join(r"Packages\Microsoft.Limitless_8wekyb3d8bbwe\LocalCache"));
    }
    v.into_iter().filter(|d| d.is_dir()).collect()
}

/// Reads `InstalledPackagesPath "G:\MSFS2024 Downloads"` from UserCfg.opt.
fn packages_path(usercfg: &str) -> Option<PathBuf> {
    let line = usercfg.lines().find(|l| l.trim_start().starts_with("InstalledPackagesPath"))?;
    let start = line.find('"')? + 1;
    let end = start + line[start..].find('"')?;
    Some(PathBuf::from(&line[start..end]))
}

/// Every Community folder in use.
pub fn community_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = config_dirs()
        .iter()
        .filter_map(|d| std::fs::read_to_string(d.join("UserCfg.opt")).ok())
        .filter_map(|s| packages_path(&s))
        .map(|p| p.join("Community"))
        .filter(|p| p.is_dir())
        .collect();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_packages_path() {
        let cfg = "Version 1\r\nInstalledPackagesPath \"G:\\MSFS2024 Downloads\"\r\n";
        assert_eq!(packages_path(cfg), Some(PathBuf::from(r"G:\MSFS2024 Downloads")));
        assert_eq!(packages_path("nothing"), None);
    }
}
