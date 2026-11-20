//! The launcher window: a 400x240 pixel canvas scaled up crisply, redrawn at 60 fps.

mod anim;
pub mod demo;
mod font;
mod gfx;
mod scene;
pub mod sound;
mod widgets;

use crate::config::Config;
use crate::{prism, server, sync, PACK_URL};
use anim::in_out_sine;
use gfx::{bayer, Canvas, H, W};
use scene::Scene;
use sound::Sfx;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use widgets::*;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

const FRAME: Duration = Duration::from_nanos(16_666_667);
const SERVER_TXT: &str = "https://raw.githubusercontent.com/menma4ever/MenmaLauncher/main/server.txt";

#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Main,
    NeedPrism,
}

enum Run {
    Idle,
    Working { stage: String, finished: bool },
    Launched { at: f32 },
    Failed { msg: String },
}

enum Msg {
    Stage(String),
    Sync(sync::Progress),
    Launched,
    Failed(String),
}

enum ServerMsg {
    NoAddress,
    Status { addr: String, status: Option<server::Status> },
    Head { uuid: String, img: [u32; 64] },
}

pub struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    canvas: Canvas,
    snapshot: Canvas,
    trans_from: Option<(Canvas, f32)>,
    scene: Scene,
    start: Instant,
    last: Instant,
    next_frame: Instant,
    mouse: Mouse,
    released: bool,
    view: (i32, i32, i32),
    screen: Screen,
    play: PlayButton,
    bar: UpdateBar,
    card: PackCard,
    mascot: Mascot,
    btn_prism: Button,
    btn_copy: Button,
    btn_server: Button,
    btn_select: Button,
    btn_download: Button,
    run: Run,
    rx: Option<Receiver<Msg>>,
    config: Config,
    prism_exe: Option<PathBuf>,
    pack: Option<sync::PackInfo>,
    pack_rx: Receiver<sync::PackInfo>,
    srv_rx: Receiver<ServerMsg>,
    srv_view: ServerView,
    srv_addr: String,
    heads: Vec<Head>,
    head_cache: HashMap<String, [u32; 64]>,
    want_heads: Sender<String>,
    host_dir: Option<PathBuf>,
    host_tx: Sender<HostMsg>,
    host_rx: Receiver<HostMsg>,
    host_running: bool,
    host_busy: bool,
    toast: Option<(String, f32)>,
    pack_url: String,
    demo: Option<demo::Kind>,
    demo_step: usize,
    demo_down: bool,
    pending_launch: bool,
    /// Background mod download: None = not started, Some(false) = running, Some(true) = ready.
    mods_ready: Option<bool>,
    prep_rx: Option<Receiver<Result<(), String>>>,
    prep_retry_at: f32,
}

enum HostMsg {
    Running(bool),
    Stage(String),
    Done(Result<(), String>),
}

/// Host PC only: report every 2 s whether our server is up (for the button label).
fn spawn_host_watch(tx: Sender<HostMsg>) {
    thread::spawn(move || loop {
        if tx.send(HostMsg::Running(crate::host::server_running())).is_err() {
            return;
        }
        thread::sleep(Duration::from_secs(2));
    });
}

fn spawn_pack_info() -> Receiver<sync::PackInfo> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Ok(info) = sync::pack_info(PACK_URL) {
            let _ = tx.send(info);
        }
    });
    rx
}

/// Background poller: reads the server address from the repo, pings every 10 s,
/// and fetches skin faces on request.
fn spawn_server_poller() -> (Receiver<ServerMsg>, Sender<String>) {
    let (tx, rx) = mpsc::channel();
    let (head_tx, head_rx) = mpsc::channel::<String>();
    let tx2 = tx.clone();
    thread::spawn(move || {
        for uuid in head_rx {
            if let Ok(img) = server::fetch_head(&uuid) {
                let _ = tx2.send(ServerMsg::Head { uuid, img });
            }
        }
    });
    thread::spawn(move || {
        let agent = sync::http_agent();
        let mut addrs: Vec<String> = Vec::new();
        let mut last_addr_check = Instant::now() - Duration::from_secs(3600);
        loop {
            if last_addr_check.elapsed() > Duration::from_secs(60) {
                last_addr_check = Instant::now();
                if let Ok(mut r) = agent.get(&sync::fresh_url(&agent, SERVER_TXT)).call()
                    && r.status() == 200
                        && let Ok(s) = r.body_mut().read_to_string() {
                            // One address per line, e.g. Tailscale first, playit second.
                            addrs = s
                                .lines()
                                .map(str::trim)
                                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                                .map(String::from)
                                .collect();
                        }
            }
            let msg = if addrs.is_empty() {
                ServerMsg::NoAddress
            } else {
                match server::ping_fastest(&addrs, Duration::from_secs(4)) {
                    Some((a, st)) => ServerMsg::Status { addr: a, status: Some(st) },
                    None => ServerMsg::Status { addr: addrs[addrs.len() - 1].clone(), status: None },
                }
            };
            if tx.send(msg).is_err() {
                return;
            }
            thread::sleep(Duration::from_secs(10));
        }
    });
    (rx, head_tx)
}

impl App {
    pub fn new(demo: Option<demo::Kind>) -> App {
        sound::init();
        let config = Config::load();
        let prism_exe = prism::find(config.prism_exe.as_deref());
        let host = crate::host::host_dir();
        let (host_tx, host_rx) = mpsc::channel();
        if host.is_some() {
            spawn_host_watch(host_tx.clone());
        }
        let (srv_rx, want_heads) = if demo == Some(demo::Kind::Server) {
            // Demo players are fed in by the script instead of a real server.
            let (_tx, rx) = mpsc::channel();
            let (htx, _hrx) = mpsc::channel();
            (rx, htx)
        } else {
            spawn_server_poller()
        };
        let pack_url = if demo == Some(demo::Kind::Error) {
            // A pack address that doesn't exist, to show the error path.
            PACK_URL.replace("pack.toml", "missing-pack.toml")
        } else {
            PACK_URL.to_string()
        };
        let mut btn_server = Button::new("START SERVER", 300, 218, 90, TEAL);
        btn_server.visible = if host.is_some() { 1.0 } else { 0.0 };
        let now = Instant::now();
        App {
            window: None,
            surface: None,
            canvas: Canvas::new(),
            snapshot: Canvas::new(),
            trans_from: None,
            scene: Scene::new(),
            start: now,
            last: now,
            next_frame: now,
            mouse: Mouse::default(),
            released: false,
            view: (1, 0, 0),
            screen: if prism_exe.is_some() { Screen::Main } else { Screen::NeedPrism },
            play: PlayButton::new(140, 126),
            bar: UpdateBar::new(98, 176),
            card: PackCard::new(12, 54),
            mascot: Mascot::new(),
            btn_prism: Button::new("PRISM...", 10, 218, 60, 0x6a6590),
            btn_copy: Button::new("COPY", 352, 61, 36, 0x6a6590),
            btn_server,
            btn_select: Button::new("SELECT YOUR PRISM EXE", 110, 172, 180, GOLD),
            btn_download: Button::new("DOWNLOAD PRISM", 110, 192, 180, TEAL),
            run: Run::Idle,
            rx: None,
            config,
            prism_exe,
            pack: None,
            pack_rx: spawn_pack_info(),
            srv_rx,
            srv_view: ServerView::Checking,
            srv_addr: String::new(),
            heads: Vec::new(),
            head_cache: HashMap::new(),
            want_heads,
            host_dir: host,
            host_tx,
            host_rx,
            host_running: false,
            host_busy: false,
            toast: None,
            pack_url,
            demo,
            demo_step: 0,
            demo_down: false,
            pending_launch: false,
            mods_ready: None,
            prep_rx: None,
            prep_retry_at: 0.0,
        }
    }

    fn t(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    fn go(&mut self, s: Screen) {
        if self.screen != s {
            self.trans_from = Some((Canvas { w: W, h: H, px: self.canvas.px.clone() }, self.t()));
            self.screen = s;
        }
    }

    /// Download the pack's mods into the player's Prism right away, quietly, so
    /// Play only has to double-check. Minecraft/NeoForge themselves come from Prism.
    fn start_prep(&mut self) {
        let Some(exe) = self.prism_exe.clone() else { return };
        if self.demo.is_some() || self.prep_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.prep_rx = Some(rx);
        self.mods_ready = Some(false);
        let pack_url = self.pack_url.clone();
        thread::spawn(move || {
            let r = (|| -> Result<(), String> {
                let info = sync::pack_info(&pack_url).map_err(|e| e.to_string())?;
                let instances = prism::instances_dir(&prism::data_dir(&exe));
                let prepared = prism::prepare_instance(&instances, &info.minecraft, info.neoforge.as_deref())
                    .map_err(|e| e.to_string())?;
                let (ptx, _prx) = mpsc::channel();
                let opts = sync::Options { pack_url, dest: prepared.game_dir, side: sync::Side::Client, workers: 12 };
                sync::sync(&opts, &ptx).map(|_| ()).map_err(|e| e.to_string())
            })();
            let _ = tx.send(r);
        });
    }

    fn start_play(&mut self) {
        let Some(exe) = self.prism_exe.clone() else {
            self.go(Screen::NeedPrism);
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.bar.reset();
        self.run = Run::Working { stage: "CHECKING PACK...".into(), finished: false };
        let server = if self.srv_addr.is_empty() { None } else { Some(self.srv_addr.clone()) };
        let pack_url = self.pack_url.clone();
        thread::spawn(move || {
            let fail = |tx: &Sender<Msg>, e: String| {
                let _ = tx.send(Msg::Failed(e));
            };
            let info = match sync::pack_info(&pack_url) {
                Ok(i) => i,
                Err(e) => return fail(&tx, format!("Can't reach the pack: {e}")),
            };
            let instances = prism::instances_dir(&prism::data_dir(&exe));
            let prepared = match prism::prepare_instance(&instances, &info.minecraft, info.neoforge.as_deref()) {
                Ok(p) => p,
                Err(e) => return fail(&tx, format!("Can't set up the Menma instance: {e}")),
            };
            let _ = tx.send(Msg::Stage("UPDATING MODS...".into()));
            let (ptx, prx) = mpsc::channel();
            let fwd = tx.clone();
            let forward = thread::spawn(move || {
                for p in prx {
                    let _ = fwd.send(Msg::Sync(p));
                }
            });
            let opts = sync::Options { pack_url, dest: prepared.game_dir, side: sync::Side::Client, workers: 12 };
            let res = sync::sync(&opts, &ptx);
            drop(ptx);
            let _ = forward.join();
            if let Err(e) = res {
                return fail(&tx, e.to_string());
            }
            let _ = tx.send(Msg::Stage("STARTING PRISM...".into()));
            match prism::launch(&exe, server.as_deref()) {
                Ok(()) => {
                    let _ = tx.send(Msg::Launched);
                }
                Err(e) => fail(&tx, format!("Prism didn't start: {e}")),
            }
        });
    }

    fn apply_server(&mut self, addr: Option<String>, status: Option<server::Status>, t: f32) {
        if let Some(a) = addr {
            self.srv_addr = a;
        }
        match status {
            None => {
                self.srv_view = ServerView::Offline;
                for h in &mut self.heads {
                    h.leaving.get_or_insert(t);
                }
            }
            Some(s) => {
                self.srv_view = ServerView::Online { ping: s.ping_ms, online: s.online, max: s.max };
                let names: Vec<&str> = s.players.iter().map(|p| p.name.as_str()).collect();
                for h in &mut self.heads {
                    if !names.contains(&h.name.as_str()) {
                        h.leaving.get_or_insert(t);
                    }
                }
                let mut delay = 0.0;
                for p in &s.players {
                    if !self.heads.iter().any(|h| h.name == p.name && h.leaving.is_none()) {
                        let img = match self.head_cache.get(&p.uuid) {
                            Some(i) => *i,
                            None => {
                                let _ = self.want_heads.send(p.uuid.clone());
                                default_head()
                            }
                        };
                        // New faces pop in one by one.
                        self.heads.push(Head {
                            name: p.name.clone(),
                            uuid: p.uuid.clone(),
                            img,
                            born: t + delay,
                            leaving: None,
                        });
                        delay += 0.18;
                    }
                }
            }
        }
    }

    fn poll(&mut self, t: f32) {
        if let Ok(info) = self.pack_rx.try_recv() {
            self.pack = Some(info);
        }
        while let Ok(m) = self.srv_rx.try_recv() {
            match m {
                ServerMsg::NoAddress => self.srv_view = ServerView::NotSetUp,
                ServerMsg::Status { addr, status } => self.apply_server(Some(addr), status, t),
                ServerMsg::Head { uuid, img } => {
                    // Swap the placeholder for the real face (same slot, no re-pop).
                    for h in self.heads.iter_mut().filter(|h| h.uuid == uuid) {
                        h.img = img;
                    }
                    self.head_cache.insert(uuid, img);
                }
            }
        }
        self.heads.retain(|h| h.leaving.is_none_or(|l| t - l < 0.35));

        let Some(rx) = &self.rx else { return };
        let mut done = false;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Stage(s) => {
                    if let Run::Working { stage, .. } = &mut self.run {
                        *stage = s;
                    }
                }
                Msg::Sync(p) => match p {
                    sync::Progress::Plan { to_download: 0, .. } => self.bar.target = 1.0,
                    sync::Progress::FileDone { done, total, .. } => self.bar.target = done as f32 / total.max(1) as f32,
                    sync::Progress::Finished(_) => {
                        self.bar.target = 1.0;
                        if let Run::Working { finished, .. } = &mut self.run {
                            *finished = true;
                        }
                    }
                    _ => {}
                },
                Msg::Launched => {
                    // Celebrate once the bar has visibly finished, not before.
                    self.bar.target = 1.0;
                    self.pending_launch = true;
                    done = true;
                }
                Msg::Failed(e) => {
                    self.run = Run::Failed { msg: friendly(&e) };
                    self.mascot.set(Mood::Worried, t);
                    sound::play(Sfx::Error);
                    self.play.label = "RETRY".into();
                    self.play.reform();
                    done = true;
                }
            }
        }
        if done {
            self.rx = None;
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.05);
        self.last = now;
        let t = self.t();
        if let Some(k) = self.demo {
            self.drive_demo(k, t);
        }
        let hour = self.demo.and_then(|k| k.hour(t)).unwrap_or_else(Scene::clock_hours);
        let night = Scene::night_factor(hour);
        let released = std::mem::take(&mut self.released);
        let m = self.mouse;
        self.poll(t);
        if self.prep_rx.is_none() && self.mods_ready.is_none() && self.prism_exe.is_some() && t >= self.prep_retry_at {
            self.start_prep();
        }
        if let Some(rx) = &self.prep_rx
            && let Ok(r) = rx.try_recv()
        {
            self.prep_rx = None;
            if r.is_ok() {
                self.mods_ready = Some(true);
            } else {
                // Quietly try again in a minute; Play does a full sync anyway and shows errors.
                self.mods_ready = None;
                self.prep_retry_at = t + 60.0;
            }
        }
        while let Ok(h) = self.host_rx.try_recv() {
            match h {
                HostMsg::Running(r) => self.host_running = r,
                HostMsg::Stage(s) => self.toast = Some((s, t)),
                HostMsg::Done(r) => {
                    self.host_busy = false;
                    match r {
                        Ok(()) => {
                            let msg = if self.host_running { "SERVER ONLINE" } else { "SERVER STOPPED" };
                            self.toast = Some((msg.into(), t));
                            sound::play(Sfx::Cheer);
                        }
                        Err(e) => {
                            self.run = Run::Failed { msg: friendly(&e) };
                            self.mascot.set(Mood::Worried, t);
                            sound::play(Sfx::Error);
                        }
                    }
                }
            }
        }
        let (label, color) = match (self.host_busy, self.host_running) {
            (true, true) => ("STOPPING...", GOLD),
            (true, false) => ("STARTING...", GOLD),
            (false, true) => ("STOP SERVER", RED),
            (false, false) => ("START SERVER", TEAL),
        };
        self.btn_server.label = label.into();
        self.btn_server.color = color;

        // ---- update ----
        match self.screen {
            Screen::Main => {
                if self.play.update(&m, released, dt, &self.canvas) {
                    self.start_play();
                }
                self.card.update(&m, dt);
                self.btn_copy.visible = if self.srv_addr.is_empty() { 0.0 } else { 1.0 };
                if self.btn_copy.update(&m, released, dt) {
                    let msg = if copy_to_clipboard(&self.srv_addr) { "ADDRESS COPIED" } else { "COULDN'T COPY" };
                    self.toast = Some((msg.into(), t));
                }
                if self.btn_prism.update(&m, released, dt) {
                    self.go(Screen::NeedPrism);
                }
                if self.host_dir.is_some()
                    && self.btn_server.update(&m, released, dt)
                    && !self.host_busy
                    && let Some(dir) = self.host_dir.clone()
                {
                    // Start or stop the server + tunnel in the background, no windows.
                    self.host_busy = true;
                    let stopping = self.host_running;
                    let tx = self.host_tx.clone();
                    let pack_url = self.pack_url.clone();
                    thread::spawn(move || {
                        let report = |s: &str| {
                            let _ = tx.send(HostMsg::Stage(s.to_string()));
                        };
                        let r = if stopping {
                            crate::host::stop(&dir, &report)
                        } else {
                            crate::host::start(&dir, &pack_url, &report)
                        };
                        let _ = tx.send(HostMsg::Running(crate::host::server_running()));
                        let _ = tx.send(HostMsg::Done(r));
                    });
                }
            }
            Screen::NeedPrism => {
                if self.btn_select.update(&m, released, dt)
                    && let Some(p) = pick_exe() {
                        if prism::looks_like_prism(&p) {
                            self.config.prism_exe = Some(p.clone());
                            self.config.save();
                            self.prism_exe = Some(p);
                            self.prep_rx = None;
                            self.start_prep();
                            self.toast = Some(("PRISM FOUND".into(), t));
                            self.go(Screen::Main);
                        } else {
                            self.toast = Some(("THAT'S NOT PRISMLAUNCHER.EXE".into(), t));
                            sound::play(Sfx::Error);
                        }
                    }
                if self.btn_download.update(&m, released, dt) {
                    prism::open_url(prism::DOWNLOAD_PAGE);
                }
                if self.prism_exe.is_some() && released && !m.over(80, 86, 240, 128) {
                    self.go(Screen::Main);
                }
            }
        }
        let working = matches!(self.run, Run::Working { .. });
        self.bar.busy = working && self.bar.target <= 0.0;
        self.bar.update(t, dt, working || matches!(self.run, Run::Launched { at } if t - at < 2.5));
        if self.pending_launch && self.bar.settled(t) {
            self.pending_launch = false;
            self.run = Run::Launched { at: t };
            self.mascot.set(Mood::Cheer, t);
            sound::play(Sfx::Cheer);
            self.toast = Some(("GAME STARTING IN PRISM".into(), t));
        }
        if let Run::Launched { at } = self.run
            && t - at > 3.0 {
                self.run = Run::Idle;
                self.play.label = "PLAY".into();
                self.play.reform();
                self.mascot.set(Mood::Idle, t);
                // The game is starting: get out of the way (still in the taskbar).
                // (Demos keep the window up for recording, unless a test asks otherwise.)
                if (self.demo.is_none() || std::env::var_os("MENMA_TEST_MINIMIZE").is_some())
                    && let Some(w) = &self.window
                {
                    w.set_minimized(true);
                }
            }
        self.mascot.update(t, dt, &m, 76.0);
        self.scene.update(dt, t, m.inside.then_some((m.x, m.y)), night);

        // ---- draw ----
        let c = &mut self.canvas;
        self.scene.draw(c, t, hour);
        c.text_sh(12, 12, "MENMA", INK, 3, 1.0);
        c.text_sh(13, 36, "MODPACK LAUNCHER", DIM, 1, 1.0);
        draw_server_card(c, 266, 8, &self.srv_view, &self.heads, t, &self.srv_addr);

        if self.screen == Screen::Main {
            let (l1, l2) = match &self.pack {
                Some(p) => (
                    format!("{} NEOFORGE", p.minecraft),
                    match self.mods_ready {
                        Some(true) => format!("{} MODS - READY", p.mods),
                        Some(false) => format!("{} MODS - DOWNLOADING", p.mods),
                        None => format!("{} MODS", p.mods),
                    },
                ),
                None => ("LOADING...".into(), String::new()),
            };
            let title = match &self.pack {
                Some(p) => format!("{} PACK", p.name.to_uppercase()),
                None => "MENMA PACK".into(),
            };
            self.card.draw(c, &title, &l1, &l2, 1.0);

            // Play block (with dissolve when it reforms).
            self.snapshot.px.copy_from_slice(&c.px);
            self.play.draw(c, &m, t);
            self.play.dissolve_mask(c, &self.snapshot);
            self.mascot.draw(c, 64, 128, t);
            self.bar.draw(c, t);

            let status = match &self.run {
                Run::Idle => String::new(),
                Run::Working { stage, finished } => {
                    if *finished && !self.bar.full() { "UPDATING MODS...".into() } else { stage.clone() }
                }
                Run::Launched { .. } => "HAVE FUN!".into(),
                Run::Failed { .. } => String::new(),
            };
            if !status.is_empty() {
                c.text_center(200, 196, &status, INK, 1, 1.0);
            }
            if let Run::Failed { msg } = &self.run {
                let lines = font::wrap(msg, 54);
                let h = 8 + lines.len().min(3) as i32 * 9;
                c.panel(36, 172, 328, h + 2, 0x2a0f16, RED, 0.92);
                for (i, l) in lines.iter().take(3).enumerate() {
                    c.text_sh(42, 177 + i as i32 * 9, l, 0xffc9c4, 1, 1.0);
                }
            }
            self.btn_prism.draw(c, &m);
            self.btn_copy.draw(c, &m);
            if self.host_dir.is_some() {
                self.btn_server.draw(c, &m);
            }
        } else {
            self.mascot.draw(c, 30, 176, t);
            c.panel(80, 86, 240, 128, PANEL, BORDER, 0.95);
            c.text_center(200, 96, "FIND PRISM LAUNCHER", GOLD, 1, 1.0);
            let text = "Menma starts the game through your own Prism Launcher. Pick prismlauncher.exe, or download Prism first.";
            for (i, l) in font::wrap(text, 36).iter().enumerate() {
                c.text_center(200, 112 + i as i32 * 10, l, INK, 1, 1.0);
            }
            if let Some(p) = &self.prism_exe {
                let mut s = p.display().to_string();
                if s.len() > 36 {
                    s = format!("...{}", &s[s.len() - 33..]);
                }
                c.text_center(200, 158, &s, DIM, 1, 1.0);
            }
            self.btn_select.draw(c, &m);
            self.btn_download.draw(c, &m);
        }

        if let Some((msg, at)) = &self.toast {
            let age = t - at;
            if age > 2.6 {
                self.toast = None;
            } else {
                let a = in_out_sine((age / 0.25).min(1.0)) * in_out_sine(((2.6 - age) / 0.4).min(1.0));
                let y = 222 - (in_out_sine((age / 0.3).min(1.0)) * 6.0) as i32;
                let w = font::width(msg, 1) + 12;
                c.panel(200 - w / 2, y - 3, w, 13, 0x101020, GOLD, a);
                c.text_center(200, y, msg, INK, 1, a);
            }
        }

        if let Some(k) = self.demo {
            let (x, y, down) = k.mouse(t);
            draw_cursor(c, x as i32, y as i32 + if down { 1 } else { 0 }, down);
        }

        // Screen change: ordered pixel dissolve from the old frame.
        if let Some((from, at)) = &self.trans_from {
            let k = in_out_sine(((t - at) / 0.45).min(1.0));
            if k >= 1.0 {
                self.trans_from = None;
            } else {
                for y in 0..H {
                    for x in 0..W {
                        if bayer(x / 3, y / 3) > k {
                            c.put(x, y, from.get(x, y));
                        }
                    }
                }
            }
        }

        // ---- present ----
        let (Some(win), Some(surface)) = (&self.window, &mut self.surface) else { return };
        let size = win.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if surface.resize(w, h).is_err() {
            return;
        }
        if let Ok(mut buf) = surface.buffer_mut() {
            self.view = self.canvas.present(&mut buf, size.width, size.height, 0x07070f);
            let _ = buf.present();
        }
    }
}

impl App {
    fn drive_demo(&mut self, k: demo::Kind, t: f32) {
        let (x, y, down) = k.mouse(t);
        self.mouse = Mouse { x, y, inside: true, down };
        if self.demo_down && !down {
            self.released = true;
        }
        self.demo_down = down;
        if k == demo::Kind::Server {
            let names = ["Menma", "Kaito", "Rin", "Sora"];
            let roster: &[(f32, Option<&[usize]>, u32)] = &[
                (1.0, Some(&[]), 38),
                (3.0, Some(&[0]), 41),
                (4.0, Some(&[0, 1]), 36),
                (4.6, Some(&[0, 1, 2]), 52),
                (5.2, Some(&[0, 1, 2, 3]), 47),
                (8.0, Some(&[0, 2, 3]), 95),
                (10.0, None, 0),
                (11.5, Some(&[0, 2]), 44),
            ];
            for i in 0..names.len() {
                self.head_cache.insert(format!("demo-{i}"), demo::fake_face(i as u32 * 7 + 3));
            }
            while self.demo_step < roster.len() && t >= roster[self.demo_step].0 {
                let (_, who, ping) = roster[self.demo_step];
                self.demo_step += 1;
                let status = who.map(|ids| server::Status {
                    ping_ms: ping,
                    online: ids.len() as u32,
                    max: 10,
                    motd: String::new(),
                    version: String::new(),
                    players: ids
                        .iter()
                        .map(|&i| server::Player { name: names[i].into(), uuid: format!("demo-{i}") })
                        .collect(),
                });
                self.apply_server(Some("menma.example".into()), status, t);
            }
        }
        if t > k.length() {
            std::process::exit(0);
        }
    }
}

/// Shorten errors for the screen: web addresses become just their host name.
fn friendly(msg: &str) -> String {
    msg.split_whitespace()
        .map(|w| {
            let core = w.trim_start_matches('(');
            if let Some(rest) = core.strip_prefix("https://").or_else(|| core.strip_prefix("http://")) {
                rest.split('/').next().unwrap_or(rest).to_string()
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn friendly_errors_hide_long_urls() {
        let m = super::friendly("Can't reach the pack: download failed: https://raw.githubusercontent.com/a/b/c.toml (http status: 404)");
        assert_eq!(m, "Can't reach the pack: download failed: raw.githubusercontent.com (http status: 404)");
    }
}

/// Put text on the Windows clipboard (for the server address).
fn copy_to_clipboard(text: &str) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
        use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
        const CF_UNICODETEXT: u32 = 13;
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        EmptyClipboard();
        let bytes = wide.len() * 2;
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
        let mut ok = false;
        if !h.is_null() {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                GlobalUnlock(h);
                // On success the clipboard owns the memory.
                ok = !SetClipboardData(CF_UNICODETEXT, h as _).is_null();
            }
        }
        CloseClipboard();
        ok
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        false
    }
}

fn pick_exe() -> Option<PathBuf> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::Controls::Dialogs::{
            GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
        };
        let filter: Vec<u16> = "Prism Launcher (prismlauncher.exe)\0prismlauncher.exe\0Programs (*.exe)\0*.exe\0\0"
            .encode_utf16()
            .collect();
        let title: Vec<u16> = "Select your prismlauncher.exe\0".encode_utf16().collect();
        let mut file = vec![0u16; 1024];
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.lpstrFilter = filter.as_ptr();
        ofn.lpstrFile = file.as_mut_ptr();
        ofn.nMaxFile = file.len() as u32;
        ofn.lpstrTitle = title.as_ptr();
        ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
        if GetOpenFileNameW(&mut ofn) != 0 {
            let len = file.iter().position(|&c| c == 0).unwrap_or(0);
            return Some(PathBuf::from(String::from_utf16_lossy(&file[..len])));
        }
        None
    }
    #[cfg(not(windows))]
    None
}

fn dark_title_bar(win: &Window) {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = win.window_handle()
            && let RawWindowHandle::Win32(h) = h.as_raw() {
                let on: i32 = 1;
                unsafe {
                    windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
                        h.hwnd.get() as _,
                        20, // DWMWA_USE_IMMERSIVE_DARK_MODE
                        &on as *const i32 as _,
                        4,
                    );
                }
            }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Menma Launcher")
            .with_inner_size(PhysicalSize::new(W as u32 * 3, H as u32 * 3))
            .with_min_inner_size(PhysicalSize::new(W as u32, H as u32))
            // Demos stay on top so the recording sees only the launcher.
            .with_window_level(if self.demo.is_some() {
                winit::window::WindowLevel::AlwaysOnTop
            } else {
                winit::window::WindowLevel::Normal
            });
        let win = Rc::new(el.create_window(attrs).expect("window"));
        dark_title_bar(&win);
        let ctx = softbuffer::Context::new(win.clone()).expect("softbuffer context");
        self.surface = Some(softbuffer::Surface::new(&ctx, win.clone()).expect("surface"));
        self.window = Some(win);
        self.next_frame = Instant::now();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::CursorMoved { position, .. } => {
                let (s, ox, oy) = self.view;
                self.mouse.x = (position.x as f32 - ox as f32) / s as f32;
                self.mouse.y = (position.y as f32 - oy as f32) / s as f32;
                self.mouse.inside = true;
            }
            WindowEvent::CursorLeft { .. } => self.mouse.inside = false,
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => self.mouse.down = true,
                ElementState::Released => {
                    if self.mouse.down {
                        self.released = true;
                    }
                    self.mouse.down = false;
                }
            },
            WindowEvent::RedrawRequested => self.frame(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_frame {
            if let Some(w) = &self.window {
                // Windows sends no redraws to a minimized window; keep the launcher
                // working anyway (Play progress, server button) by running the frame
                // directly. Drawing is skipped while the window has no size.
                if w.is_minimized() == Some(true) {
                    self.frame();
                } else {
                    w.request_redraw();
                }
            }
            self.next_frame += FRAME;
            if self.next_frame < now {
                self.next_frame = now + FRAME;
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

pub fn run(demo: Option<demo::Kind>) {
    let el = EventLoop::new().expect("event loop");
    let mut app = App::new(demo);
    let _ = el.run_app(&mut app);
}

#[cfg(test)]
mod clipboard_test {
    #[test]
    #[ignore] // touches the real clipboard; run on purpose
    fn copies_text() {
        assert!(super::copy_to_clipboard("100.68.16.55"));
    }
}
