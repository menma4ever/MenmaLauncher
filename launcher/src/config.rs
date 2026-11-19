//! Small per-user settings file: %APPDATA%\MenmaLauncher\config.json.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Config {
    /// Prism exe the player picked (or we found), remembered between runs.
    pub prism_exe: Option<PathBuf>,
}

fn path() -> Option<PathBuf> {
    // MENMA_HOME lets tests and demos use their own settings folder.
    if let Some(h) = std::env::var_os("MENMA_HOME") {
        return Some(PathBuf::from(h).join("config.json"));
    }
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("MenmaLauncher").join("config.json"))
}

impl Config {
    pub fn load() -> Config {
        path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = path() {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, serde_json::to_vec_pretty(self).unwrap());
        }
    }
}
