// A window app: no console window. Command-line modes (--sync, --host, --fetch)
// attach to the terminal they were started from so their output still shows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod host;
mod prism;
mod server;
mod sync;
mod ui;

use std::path::PathBuf;

pub const PACK_URL: &str =
    "https://raw.githubusercontent.com/menma4ever/MenmaLauncher/main/pack/pack.toml";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    if args.iter().any(|a| a.starts_with("--") && a != "--demo") {
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
    }
    // Host: `--host start` / `--host stop` (same as the launcher's server button).
    if let Some(i) = args.iter().position(|a| a == "--host") {
        let Some(dir) = host::host_dir() else {
            println!("no server next to this launcher");
            std::process::exit(2)
        };
        let report = |s: &str| println!("{s}");
        let r = match args.get(i + 1).map(String::as_str) {
            Some("stop") => host::stop(&dir, &report),
            _ => host::start(&dir, PACK_URL, &report),
        };
        match r {
            Ok(()) => println!("ok (server running: {})", host::server_running()),
            Err(e) => {
                println!("failed: {e}");
                std::process::exit(1)
            }
        }
        return;
    }
    // Diagnostics: download one URL and report time/size (or the error).
    if let Some(i) = args.iter().position(|a| a == "--fetch") {
        let url = args.get(i + 1).cloned().unwrap_or_default();
        let t = std::time::Instant::now();
        match sync::fetch(&url) {
            Ok(b) => println!("ok {} bytes in {:.1}s", b.len(), t.elapsed().as_secs_f32()),
            Err(e) => println!("error after {:.1}s: {e}", t.elapsed().as_secs_f32()),
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--sync") {
        let dest = args.get(i + 1).map(PathBuf::from).unwrap_or_else(|| {
            eprintln!("usage: --sync <folder> [--server] [--pack <url>]");
            std::process::exit(2)
        });
        let side = if args.iter().any(|a| a == "--server") { sync::Side::Server } else { sync::Side::Client };
        let pack_url = args
            .iter()
            .position(|a| a == "--pack")
            .and_then(|j| args.get(j + 1).cloned())
            .unwrap_or_else(|| PACK_URL.to_string());
        let started = std::time::Instant::now();
        match sync::sync_print(&sync::Options { pack_url, dest, side, workers: 12 }) {
            Ok(s) => println!(
                "done in {:.1}s: {} downloaded, {} up to date, {} removed (Minecraft {}, NeoForge {})",
                started.elapsed().as_secs_f32(),
                s.downloaded,
                s.up_to_date,
                s.removed,
                s.minecraft,
                s.neoforge.unwrap_or_default()
            ),
            Err(e) => {
                eprintln!("sync failed: {e}");
                std::process::exit(1)
            }
        }
        return;
    }
    let demo = args
        .iter()
        .position(|a| a == "--demo")
        .and_then(|i| args.get(i + 1))
        .and_then(|k| ui::demo::Kind::parse(k));
    ui::run(demo);
}
