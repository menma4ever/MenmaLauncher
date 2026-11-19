Minecraft modpack launcher, full plan (from Menma via Claude chat). Goal: friends get my exact Prism setup: pick a pack, Update, Play. Normal Microsoft login only, no offline or nickname mode.

1) Packs from my Prism: read all my Prism instances (all 4, e.g. the one with Dweller's horror mod) from a COPY. Never modify or delete my originals. NEVER copy accounts.json or any login tokens. Each instance becomes a packwiz pack (mods with versions, hashes and Modrinth/CurseForge links, plus configs), hosted on GitHub. If a mod blocks outside downloads, use its Modrinth version, else the updater shows a one-time link.
2) My publish flow: change mods or make a new instance in Prism, then one Publish command, and it updates or appears for friends.
3) Friends' side: our small Rust launcher (a few MB) is the UI; a portable Prism runs hidden behind it and does the actual launching (packwiz-installer as pre-launch). Pack list like Prism's instance list, Update and Play, launcher self-update, auto Java 21, auto RAM per PC, adds my server to their multiplayer list, crash-log copy button. No code-signing cert for now.
4) Skins only on my server: a client skin mod in the pack that loads skins from my own skin store (CustomSkinLoader-type) plus a small skin store on my PC; the launcher's Change skin button uploads a PNG there. Their real account skins stay untouched.
5) Speed and graphics: Distant Horizons, shaders and speed mods (Sodium-style renderer, ModernFix, FerriteCore or equivalents). Presets Low/Medium/High, auto-picked from each PC's RAM and GPU; Distant Horizons and shaders off on Low. FIRST check Forge 1.21.1 vs NeoForge: if my mods exist on NeoForge, recommend switching (better Sodium/Iris support), and ask me before switching.
6) Hosting: server on my PC with a playit.gg tunnel, so friends join with a simple address and install nothing extra.

Order: get it playable first (step 1 + packwiz + playit.gg), then the Rust launcher, skins, presets. Test each step for real and report one line per step.

## Decisions so far (2026-11-20)
- Loader: NeoForge 1.21.1 (21.1.267). Pack lives in pack/ (packwiz), public repo menma4ever/MenmaLauncher.
- Prism: never bundle any Prism exe. The launcher auto-finds Prism in the usual install folders (%APPDATA%\PrismLauncher, %LOCALAPPDATA%\Programs\PrismLauncher, Program Files, Scoop, portable folders next to the launcher); if not found it asks "Select your Prism exe", remembers the path, and offers a "Download Prism" button that opens the official site (prismlauncher.org).
- Clean Prism setup (theme, pe_dark icons, cat background, Menma pack instance; no accounts, tokens, secrets, worlds, logs, screenshots, cache, skins or Mojang files) is local only: local/Menma-Prism-setup.zip, git-ignored, never pushed. The cat image is third-party art: keep it private.
- Fast updates: the launcher syncs the pack itself in Rust before starting Prism: parallel downloads (about 8-16 at once), sha hash check per file, skip files already correct, retry failed ones. packwiz-installer (already 10 parallel downloads) stays as the pre-launch fallback for plain-Prism users.
- Start server: host-only button (shown only when server/ exists) runs scripts/start-server.bat: Menma server + standalone playit.gg agent, never twice.

## Launcher UI: full pixel style (strict spec)
Quality bar: smooth 60 fps, every motion eased (no linear or instant jumps), pixel-perfect integer scaling (no blurry pixels), nothing janky while downloading. Dark theme, free pixel font (OFL-licensed), our own click sounds, small original pixel mascot. No Mojang files.

Background (layered, parallax):
- Back: slowly rotating 360 panorama of my Menma world.
- Middle: drifting pixel clouds, sun and moon on a real day/night cycle, stars at night.
- Front: a few floating particles (leaves; fireflies at night) that react slightly to the mouse.

Micro-interactions:
- Buttons press down 1-2 px with a click sound and a short pixel flash.
- Big Play button breathes slowly; glows on hover; on click it breaks like a block, then the game starts.
- Update bar fills block by block, each block dropping in with a tiny bounce; pixel percentage counter.
- Pack card: hover tilts it slightly; switching cards slides with a pixel wipe.
- Server card: online/offline, ping indicator that pulses, friends' pixel skin heads pop in one by one when they join.
- Screen changes: pixel dissolve or wipe, never a hard cut.
- Mascot: idles (blinks, looks around), cheers when the update finishes, looks worried on errors.

Proof before saying done: record a short screen video of every animation, send it to Menma, fix what he flags.
