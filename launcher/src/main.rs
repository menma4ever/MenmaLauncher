mod config;
mod prism;
mod server;
mod sync;
mod ui;

use std::path::PathBuf;

pub const PACK_URL: &str =
    "https://raw.githubusercontent.com/menma4ever/MenmaLauncher/main/pack/pack.toml";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
