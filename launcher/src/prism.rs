//! Everything about the player's own Prism Launcher: finding it, preparing the
//! Menma instance inside it, and asking it to launch. We never ship Prism and never
//! touch its accounts; Microsoft sign-in happens in Prism itself.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const INSTANCE: &str = "Menma";
pub const DOWNLOAD_PAGE: &str = "https://prismlauncher.org/download/?platform=windows";

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

/// Usual install locations, most likely first.
pub fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = env_path("LOCALAPPDATA") {
        v.push(p.join("Programs").join("PrismLauncher").join("prismlauncher.exe"));
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(p) = env_path(var) {
            v.push(p.join("PrismLauncher").join("prismlauncher.exe"));
            v.push(p.join("Prism Launcher").join("prismlauncher.exe"));
        }
    }
    if let Some(p) = env_path("USERPROFILE") {
        v.push(p.join("scoop").join("apps").join("prismlauncher").join("current").join("prismlauncher.exe"));
    }
    if let Some(p) = env_path("APPDATA") {
        v.push(p.join("PrismLauncher").join("prismlauncher.exe"));
    }
    // A portable Prism sitting next to this launcher.
    if let Ok(me) = std::env::current_exe()
        && let Some(dir) = me.parent() {
            v.push(dir.join("prismlauncher.exe"));
            v.push(dir.join("PrismLauncher").join("prismlauncher.exe"));
        }
    v
}

pub fn looks_like_prism(exe: &Path) -> bool {
    exe.is_file()
        && exe
            .file_name()
            .map(|n| n.to_string_lossy().eq_ignore_ascii_case("prismlauncher.exe"))
            .unwrap_or(false)
}

/// The remembered path wins if it still exists, then the usual folders.
pub fn find(remembered: Option<&Path>) -> Option<PathBuf> {
    remembered
        .filter(|p| looks_like_prism(p))
        .map(Path::to_path_buf)
        .or_else(|| candidates().into_iter().find(|p| looks_like_prism(p)))
}

/// Where Prism keeps its data: next to the exe when portable, else %APPDATA%.
pub fn data_dir(exe: &Path) -> PathBuf {
    let dir = exe.parent().unwrap_or(Path::new(".")).to_path_buf();
    if dir.join("portable.txt").exists() {
        dir
    } else {
        env_path("APPDATA").unwrap_or(dir.clone()).join("PrismLauncher")
    }
}

fn read_ini_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().to_string())
    })
}

pub fn instances_dir(data: &Path) -> PathBuf {
    let cfg = fs::read_to_string(data.join("prismlauncher.cfg")).unwrap_or_default();
    match read_ini_value(&cfg, "InstanceDir").filter(|s| !s.is_empty()) {
        Some(d) if Path::new(&d).is_absolute() => PathBuf::from(d),
        Some(d) => data.join(d),
        None => data.join("instances"),
    }
}

/// Set `key=value` in an INI-style file's [General] section (adding it if missing).
fn set_ini(text: &str, key: &str, value: &str) -> String {
    let mut out = Vec::new();
    let mut found = false;
    for line in text.lines() {
        match line.split_once('=') {
            Some((k, _)) if k.trim() == key => {
                out.push(format!("{key}={value}"));
                found = true;
            }
            _ => out.push(line.to_string()),
        }
    }
    if !found {
        if !out.iter().any(|l| l.trim() == "[General]") {
            out.insert(0, "[General]".into());
        }
        let at = out.iter().position(|l| l.trim() == "[General]").unwrap() + 1;
        out.insert(at, format!("{key}={value}"));
    }
    out.join("\n") + "\n"
}

/// Memory for the game, picked from this PC's RAM (MB).
pub fn auto_memory_mb() -> u32 {
    let total_gb = total_ram_bytes() / (1024 * 1024 * 1024);
    match total_gb {
        0..=8 => 3072,
        9..=16 => 4096,
        _ => 6144,
    }
}

#[cfg(windows)]
fn total_ram_bytes() -> u64 {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut m) } != 0 { m.ullTotalPhys } else { 8 << 30 }
}

#[cfg(not(windows))]
fn total_ram_bytes() -> u64 {
    8 << 30
}

pub struct Prepared {
    /// The game folder the pack syncs into.
    pub game_dir: PathBuf,
    /// True when the instance was just made (kept for callers and tests).
    #[allow(dead_code)]
    pub created: bool,
}

/// Create or update the Menma instance so Prism runs the pack's Minecraft + NeoForge.
pub fn prepare_instance(
    instances: &Path,
    minecraft: &str,
    neoforge: Option<&str>,
) -> std::io::Result<Prepared> {
    let dir = instances.join(INSTANCE);
    let created = !dir.join("instance.cfg").exists();
    fs::create_dir_all(&dir)?;

    let cfg_path = dir.join("instance.cfg");
    let mut cfg = fs::read_to_string(&cfg_path)
        .unwrap_or_else(|_| "[General]\nConfigVersion=1.2\nInstanceType=OneSix\niconKey=default\n".into());
    cfg = set_ini(&cfg, "name", INSTANCE);
    cfg = set_ini(&cfg, "AutomaticJava", "true");
    if created {
        let mem = auto_memory_mb();
        cfg = set_ini(&cfg, "OverrideMemory", "true");
        cfg = set_ini(&cfg, "MinMemAlloc", "1024");
        cfg = set_ini(&cfg, "MaxMemAlloc", &mem.to_string());
    }
    // Java on some Windows PCs (VPN/network tools installed) fails with "failed to open
    // a new selector" unless its internal socket files go to a short, simple folder.
    let fix_dir = env_path("ProgramData")
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("MenmaLauncher")
        .join("tmp");
    let _ = fs::create_dir_all(&fix_dir);
    let args = read_ini_value(&cfg, "JvmArgs").unwrap_or_default();
    if !args.contains("jdk.net.unixdomain.tmpdir") {
        let flag = format!("-Djdk.net.unixdomain.tmpdir={}", fix_dir.display());
        let joined = if args.trim().is_empty() { flag } else { format!("{} {flag}", args.trim()) };
        cfg = set_ini(&cfg, "JvmArgs", &joined);
        cfg = set_ini(&cfg, "OverrideJavaArgs", "true");
    }
    // This launcher syncs the pack itself, so the packwiz pre-launch step is not needed.
    if read_ini_value(&cfg, "PreLaunchCommand").is_some_and(|c| c.contains("packwiz-installer")) {
        cfg = set_ini(&cfg, "PreLaunchCommand", "");
        cfg = set_ini(&cfg, "OverrideCommands", "false");
    }
    fs::write(&cfg_path, cfg)?;

    let mut components = vec![serde_json::json!({
        "important": true, "uid": "net.minecraft", "version": minecraft
    })];
    if let Some(nf) = neoforge {
        components.push(serde_json::json!({ "uid": "net.neoforged", "version": nf }));
    }
    let wanted = serde_json::json!({ "components": components, "formatVersion": 1 });
    let pack_path = dir.join("mmc-pack.json");
    let current: Option<serde_json::Value> =
        fs::read(&pack_path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    if !versions_match(current.as_ref(), minecraft, neoforge) {
        fs::write(&pack_path, serde_json::to_vec_pretty(&wanted).unwrap())?;
    }

    // Prism uses "minecraft" (newer) or ".minecraft" (older instances).
    let game_dir = if dir.join(".minecraft").is_dir() && !dir.join("minecraft").is_dir() {
        dir.join(".minecraft")
    } else {
        dir.join("minecraft")
    };
    fs::create_dir_all(&game_dir)?;
    Ok(Prepared { game_dir, created })
}

fn versions_match(current: Option<&serde_json::Value>, mc: &str, nf: Option<&str>) -> bool {
    let Some(comps) = current.and_then(|c| c.get("components")).and_then(|c| c.as_array()) else {
        return false;
    };
    let ver = |uid: &str| {
        comps
            .iter()
            .find(|c| c.get("uid").and_then(|u| u.as_str()) == Some(uid))
            .and_then(|c| c.get("version"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    ver("net.minecraft").as_deref() == Some(mc) && ver("net.neoforged").as_deref() == nf
}

/// Ask Prism to start the instance (and optionally join a server once in game).
pub fn launch(exe: &Path, server: Option<&str>) -> std::io::Result<()> {
    let mut cmd = Command::new(exe);
    cmd.arg("--launch").arg(INSTANCE);
    if let Some(s) = server {
        cmd.arg("--server").arg(s);
    }
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn().map(|_| ())
}

pub fn open_url(url: &str) {
    let _ = Command::new("rundll32").arg("url.dll,FileProtocolHandler").arg(url).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_edits() {
        let t = "[General]\nname=Old\nPreLaunchCommand=java -jar packwiz-installer-bootstrap.jar x\n";
        let t2 = set_ini(t, "name", "Menma");
        assert!(t2.contains("name=Menma") && !t2.contains("name=Old"));
        let t3 = set_ini(&t2, "MaxMemAlloc", "4096");
        assert!(t3.starts_with("[General]\nMaxMemAlloc=4096\n"));
        assert_eq!(read_ini_value(&t3, "MaxMemAlloc").as_deref(), Some("4096"));
    }

    #[test]
    fn prepares_and_updates_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let p = prepare_instance(tmp.path(), "1.21.1", Some("21.1.267")).unwrap();
        assert!(p.created && p.game_dir.ends_with("minecraft"));
        let cfg = fs::read_to_string(tmp.path().join("Menma/instance.cfg")).unwrap();
        assert!(cfg.contains("name=Menma") && cfg.contains("MaxMemAlloc="));
        let pack = fs::read_to_string(tmp.path().join("Menma/mmc-pack.json")).unwrap();
        assert!(pack.contains("21.1.267") && pack.contains("1.21.1"));

        // Friend-zip instance with packwiz pre-launch: we clear it, keep their memory choice.
        fs::write(
            tmp.path().join("Menma/instance.cfg"),
            "[General]\nname=Menma\nMaxMemAlloc=2048\nOverrideCommands=true\nPreLaunchCommand=\"$INST_JAVA\" -jar packwiz-installer-bootstrap.jar u\n",
        )
        .unwrap();
        let p = prepare_instance(tmp.path(), "1.21.1", Some("21.1.300")).unwrap();
        assert!(!p.created);
        let cfg = fs::read_to_string(tmp.path().join("Menma/instance.cfg")).unwrap();
        assert!(cfg.contains("PreLaunchCommand=\n") && cfg.contains("MaxMemAlloc=2048"));
        assert_eq!(cfg.matches("jdk.net.unixdomain.tmpdir").count(), 1, "network fix added once");
        prepare_instance(tmp.path(), "1.21.1", Some("21.1.300")).unwrap();
        let cfg = fs::read_to_string(tmp.path().join("Menma/instance.cfg")).unwrap();
        assert_eq!(cfg.matches("jdk.net.unixdomain.tmpdir").count(), 1, "not added twice");
        assert!(cfg.contains("OverrideJavaArgs=true"));
        let pack = fs::read_to_string(tmp.path().join("Menma/mmc-pack.json")).unwrap();
        assert!(pack.contains("21.1.300"));
    }

    #[test]
    fn instance_dir_from_cfg() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(instances_dir(tmp.path()), tmp.path().join("instances"));
        fs::write(tmp.path().join("prismlauncher.cfg"), "[General]\nInstanceDir=inst2\n").unwrap();
        assert_eq!(instances_dir(tmp.path()), tmp.path().join("inst2"));
    }

    #[test]
    fn portable_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("prismlauncher.exe");
        fs::write(&exe, b"x").unwrap();
        fs::write(tmp.path().join("portable.txt"), b"").unwrap();
        assert_eq!(data_dir(&exe), tmp.path());
        assert!(looks_like_prism(&exe));
        assert_eq!(find(Some(&exe)), Some(exe.clone()));
    }
}
