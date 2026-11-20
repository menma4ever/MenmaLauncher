//! Host-only server control: start the Menma server and the playit tunnel hidden in
//! the background, and stop them properly. Stopping goes through RCON (the server's
//! own remote console), so the world is saved before the server exits.

use crate::sync;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const GAME_PORT: u16 = 25565;
const RCON_PORT: u16 = 25575;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The host's project folder: the one with server\ and tools\playit next to the launcher.
pub fn host_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().take(6).find_map(|d| {
        (d.join("server").join("user_jvm_args.txt").is_file()
            && d.join("tools").join("playit").join("playit.exe").is_file())
        .then(|| d.to_path_buf())
    })
}

fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub fn port_open(port: u16) -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

pub fn server_running() -> bool {
    port_open(GAME_PORT)
}

fn process_running(image: &str) -> bool {
    let out = hidden(Command::new("tasklist").args(["/FI", &format!("IMAGENAME eq {image}"), "/NH"]))
        .output();
    matches!(out, Ok(o) if String::from_utf8_lossy(&o.stdout).to_lowercase().contains(&image.to_lowercase()))
}

// ---------- server.properties ----------

fn read_props(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect()
}

fn set_prop(text: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| match l.split_once('=') {
            Some((k, _)) if k.trim() == key && !l.trim_start().starts_with('#') => {
                found = true;
                format!("{key}={value}")
            }
            _ => l.to_string(),
        })
        .collect();
    if !found {
        out.push(format!("{key}={value}"));
    }
    out.join("\n") + "\n"
}

fn random_password() -> String {
    // Random enough for a console only reachable from this PC/LAN.
    let mut bytes = [0u8; 16];
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Security::Cryptography::{
            BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        };
        BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 16, BCRYPT_USE_SYSTEM_PREFERRED_RNG);
    }
    hex::encode(bytes)
}

/// Turn on RCON (once) and return its password. Takes effect on the next server start.
pub fn ensure_rcon(server: &Path) -> std::io::Result<String> {
    let path = server.join("server.properties");
    let text = fs::read_to_string(&path).unwrap_or_default();
    let props = read_props(&text);
    let get = |k: &str| props.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
    if get("enable-rcon") == "true" && !get("rcon.password").is_empty() {
        return Ok(get("rcon.password"));
    }
    let pw = random_password();
    let mut t = set_prop(&text, "enable-rcon", "true");
    t = set_prop(&t, "rcon.port", &RCON_PORT.to_string());
    t = set_prop(&t, "rcon.password", &pw);
    t = set_prop(&t, "broadcast-rcon-to-ops", "false");
    fs::write(&path, t)?;
    Ok(pw)
}

fn rcon_password(server: &Path) -> Option<String> {
    let text = fs::read_to_string(server.join("server.properties")).ok()?;
    let props = read_props(&text);
    let on = props.iter().any(|(k, v)| k == "enable-rcon" && v == "true");
    let pw = props.into_iter().find(|(k, _)| k == "rcon.password").map(|(_, v)| v)?;
    (on && !pw.is_empty()).then_some(pw)
}

// ---------- RCON protocol ----------

fn rcon_packet(id: i32, kind: i32, body: &str) -> Vec<u8> {
    let mut p = Vec::new();
    let len = (4 + 4 + body.len() + 2) as i32;
    p.extend_from_slice(&len.to_le_bytes());
    p.extend_from_slice(&id.to_le_bytes());
    p.extend_from_slice(&kind.to_le_bytes());
    p.extend_from_slice(body.as_bytes());
    p.extend_from_slice(&[0, 0]);
    p
}

fn rcon_read(s: &mut TcpStream) -> Result<(i32, String), String> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).map_err(|e| e.to_string())?;
    let len = i32::from_le_bytes(len);
    if !(10..=4110).contains(&len) {
        return Err(format!("bad rcon packet length {len}"));
    }
    let mut buf = vec![0u8; len as usize];
    s.read_exact(&mut buf).map_err(|e| e.to_string())?;
    let id = i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let body = String::from_utf8_lossy(&buf[8..buf.len() - 2]).into_owned();
    Ok((id, body))
}

/// Log in and run one console command; returns the server's reply.
pub fn rcon(port: u16, password: &str, command: &str) -> Result<String, String> {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    s.write_all(&rcon_packet(1, 3, password)).map_err(|e| e.to_string())?;
    let (id, _) = rcon_read(&mut s)?;
    if id == -1 {
        return Err("RCON password rejected".into());
    }
    s.write_all(&rcon_packet(2, 2, command)).map_err(|e| e.to_string())?;
    // "stop" may close the connection before replying; that's fine.
    Ok(rcon_read(&mut s).map(|(_, b)| b).unwrap_or_default())
}

// ---------- start / stop ----------

fn java_path(host: &Path) -> Option<PathBuf> {
    // start.bat names the Java 21 to use: set "JAVA=...".
    let bat = fs::read_to_string(host.join("server").join("start.bat")).ok()?;
    bat.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix("set \"JAVA=")?;
        Some(PathBuf::from(rest.trim_end_matches('"')))
    })
}

fn neoforge_args(server: &Path, wanted: Option<&str>) -> Option<PathBuf> {
    let base = server.join("libraries").join("net").join("neoforged").join("neoforge");
    if let Some(v) = wanted {
        let p = base.join(v).join("win_args.txt");
        if p.is_file() {
            return Some(p);
        }
    }
    let mut versions: Vec<PathBuf> =
        fs::read_dir(&base).ok()?.flatten().map(|e| e.path().join("win_args.txt")).filter(|p| p.is_file()).collect();
    versions.sort();
    versions.pop()
}

/// Sync the server's mods, then start the server and playit hidden. Blocks until the
/// server is accepting players (or fails).
pub fn start(host: &Path, pack_url: &str, report: &dyn Fn(&str)) -> Result<(), String> {
    let server = host.join("server");
    if server_running() {
        report("SERVER ALREADY RUNNING");
    } else {
        ensure_rcon(&server).map_err(|e| format!("Can't write server.properties: {e}"))?;
        report("UPDATING SERVER MODS...");
        let (tx, _rx) = std::sync::mpsc::channel();
        let summary = sync::sync(
            &sync::Options { pack_url: pack_url.into(), dest: server.clone(), side: sync::Side::Server, workers: 8 },
            &tx,
        )
        .map_err(|e| e.to_string())?;
        let java = java_path(host).filter(|p| p.is_file()).ok_or("Java 21 not found (check server\\start.bat)")?;
        let args = neoforge_args(&server, summary.neoforge.as_deref()).ok_or("NeoForge server files missing")?;
        let rel = args.strip_prefix(&server).unwrap_or(&args).to_string_lossy().replace('\\', "/");
        report("STARTING SERVER...");
        hidden(
            Command::new(java)
                .arg("@user_jvm_args.txt")
                .arg(format!("@{rel}"))
                .arg("nogui")
                .current_dir(&server)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .spawn()
        .map_err(|e| format!("Couldn't start Java: {e}"))?;
    }
    if !process_running("playit.exe") {
        let dir = host.join("tools").join("playit");
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(server.join("logs").join("playit.log"))
            .or_else(|_| fs::File::create(dir.join("playit.log")))
            .map_err(|e| e.to_string())?;
        hidden(
            Command::new(dir.join("playit.exe"))
                .arg("--stdout")
                .current_dir(&dir)
                .stdin(Stdio::null())
                .stdout(log.try_clone().map_err(|e| e.to_string())?)
                .stderr(log),
        )
        .spawn()
        .map_err(|e| format!("Couldn't start playit: {e}"))?;
    }
    let deadline = Instant::now() + Duration::from_secs(240);
    while Instant::now() < deadline {
        if server_running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(700));
    }
    Err("The server didn't come up within 4 minutes (see server\\logs\\latest.log)".into())
}

/// Save and stop the server through RCON, then stop playit.
pub fn stop(host: &Path, report: &dyn Fn(&str)) -> Result<(), String> {
    let server = host.join("server");
    if server_running() {
        let pw = rcon_password(&server)
            .ok_or("This server was started without remote control. Type 'stop' in its window once; after that the launcher can stop it.")?;
        report("SAVING WORLD...");
        rcon(RCON_PORT, &pw, "stop").map_err(|e| {
            format!("Couldn't reach the server console ({e}). Type 'stop' in its window.")
        })?;
        let deadline = Instant::now() + Duration::from_secs(60);
        while server_running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(500));
        }
        if server_running() {
            return Err("The server is still shutting down; give it a moment.".into());
        }
    }
    report("STOPPING TUNNEL...");
    let _ = hidden(Command::new("taskkill").args(["/IM", "playit.exe", "/F"])).output();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn props_roundtrip() {
        let t = "#comment\nenable-rcon=false\nmotd=Hi\n";
        let t = set_prop(t, "enable-rcon", "true");
        let t = set_prop(&t, "rcon.password", "abc");
        let p = read_props(&t);
        assert!(p.contains(&("enable-rcon".into(), "true".into())));
        assert!(p.contains(&("rcon.password".into(), "abc".into())));
        assert!(t.starts_with("#comment\n"));
    }

    #[test]
    fn ensure_rcon_sets_once() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("server.properties"), "enable-rcon=false\nrcon.password=\n").unwrap();
        let a = ensure_rcon(d.path()).unwrap();
        let b = ensure_rcon(d.path()).unwrap();
        assert_eq!(a.len(), 32);
        assert_eq!(a, b, "password is kept once set");
        assert_eq!(rcon_password(d.path()), Some(a));
    }

    #[test]
    fn rcon_talks_to_a_fake_server() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let srv = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let (id, body) = rcon_read(&mut s).unwrap();
            assert_eq!(body, "pw");
            s.write_all(&rcon_packet(id, 2, "")).unwrap();
            let (id, body) = rcon_read(&mut s).unwrap();
            assert_eq!(body, "list");
            s.write_all(&rcon_packet(id, 0, "There are 0 of a max of 10 players online")).unwrap();
        });
        let r = rcon(port, "pw", "list").unwrap();
        assert!(r.contains("max of 10"));
        srv.join().unwrap();
    }

    #[test]
    fn rcon_wrong_password() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let srv = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let _ = rcon_read(&mut s).unwrap();
            s.write_all(&rcon_packet(-1, 2, "")).unwrap();
        });
        assert!(rcon(port, "bad", "stop").is_err());
        srv.join().unwrap();
    }
}
