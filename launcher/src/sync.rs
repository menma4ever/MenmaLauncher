//! Pack sync: reads a packwiz pack over HTTP and makes a local folder match it.
//!
//! Fast path: files already recorded with the right hash in our manifest are trusted
//! without rehashing. Everything that needs downloading goes through a pool of
//! worker threads, each file verified by hash before it replaces the old one.
//! Only files this sync installed earlier are ever deleted; anything the player
//! added by hand is left alone.

use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub const MANIFEST_NAME: &str = ".menma-sync.json";
const RETRIES: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Client,
    Server,
}

impl Side {
    fn wants(self, side: Option<&str>) -> bool {
        match side.unwrap_or("both") {
            "both" | "" => true,
            "client" => self == Side::Client,
            "server" => self == Side::Server,
            _ => true,
        }
    }
}

/// Progress messages for the UI. `done`/`total` count files that need downloading.
#[derive(Clone, Debug)]
pub enum Progress {
    Checking,
    Plan { to_download: usize, up_to_date: usize, to_remove: usize },
    FileDone { name: String, done: usize, total: usize },
    Removed { name: String },
    #[allow(dead_code)] // the GUI only needs the signal; the CLI uses the summary from sync()
    Finished(Summary),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub downloaded: usize,
    pub up_to_date: usize,
    pub removed: usize,
    pub minecraft: String,
    pub neoforge: Option<String>,
}

#[derive(Debug)]
pub struct SyncError(pub String);

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SyncError {}

fn err<T>(msg: impl Into<String>) -> Result<T, SyncError> {
    Err(SyncError(msg.into()))
}

// ---------- packwiz file formats ----------

#[derive(Deserialize)]
struct PackToml {
    index: IndexRef,
    versions: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct IndexRef {
    file: String,
    #[serde(rename = "hash-format")]
    hash_format: String,
    hash: String,
}

#[derive(Deserialize)]
struct IndexToml {
    #[serde(rename = "hash-format")]
    hash_format: String,
    #[serde(default)]
    files: Vec<IndexEntry>,
}

#[derive(Deserialize, Clone)]
struct IndexEntry {
    file: String,
    hash: String,
    #[serde(rename = "hash-format")]
    hash_format: Option<String>,
    #[serde(default)]
    metafile: bool,
    alias: Option<String>,
    #[serde(default)]
    preserve: bool,
}

#[derive(Deserialize)]
struct MetaToml {
    name: Option<String>,
    filename: String,
    side: Option<String>,
    download: MetaDownload,
}

#[derive(Deserialize)]
struct MetaDownload {
    url: Option<String>,
    #[serde(rename = "hash-format")]
    hash_format: String,
    hash: String,
    mode: Option<String>,
}

// ---------- our manifest ----------

#[derive(Serialize, Deserialize, Default)]
struct Manifest {
    /// Local relative path ("mods/x.jar") -> what we installed there.
    files: BTreeMap<String, Installed>,
    /// Metafile index hash -> resolved target, so unchanged mods need no metafile fetch.
    #[serde(default)]
    metafiles: BTreeMap<String, Target>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Installed {
    /// "format:hash"
    hash: String,
    /// Size on disk; a cheap check that the file wasn't damaged or replaced.
    size: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Target {
    path: String,
    url: String,
    hash_format: String,
    hash: String,
    name: String,
    side: Option<String>,
    #[serde(default)]
    preserve: bool,
}

// ---------- helpers ----------

pub fn http_agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        // rustls, trusting the Windows certificate store.
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::Rustls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        // No overall limit: big mods on slow internet must be allowed to finish.
        // Dead connections are caught by the connect/response limits and a 10 min cap.
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(600)))
        .user_agent(concat!("MenmaLauncher/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn get_bytes(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, SyncError> {
    let mut last = String::new();
    for attempt in 0..RETRIES {
        match agent.get(url).call() {
            Ok(mut resp) => {
                match resp.body_mut().with_config().limit(512 * 1024 * 1024).read_to_vec() {
                    Ok(b) => return Ok(b),
                    Err(e) => last = e.to_string(),
                }
            }
            Err(e) => last = e.to_string(),
        }
        thread::sleep(Duration::from_millis(400 * (attempt as u64 + 1)));
    }
    err(format!("download failed: {url} ({last})"))
}

/// One download with a fresh agent (used by the --fetch diagnostic).
pub fn fetch(url: &str) -> Result<Vec<u8>, SyncError> {
    get_bytes(&http_agent(), url)
}

fn get_text(agent: &ureq::Agent, url: &str) -> Result<String, SyncError> {
    String::from_utf8(get_bytes(agent, url)?).map_err(|_| SyncError(format!("not text: {url}")))
}

pub fn hash_hex(format: &str, data: &[u8]) -> Result<String, SyncError> {
    Ok(match format {
        "sha1" => hex::encode(Sha1::digest(data)),
        "sha256" => hex::encode(Sha256::digest(data)),
        "sha512" => hex::encode(Sha512::digest(data)),
        other => return err(format!("unsupported hash format: {other}")),
    })
}

fn hash_file(format: &str, path: &Path) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    hash_hex(format, &buf).ok()
}

fn tag(format: &str, hash: &str) -> String {
    format!("{format}:{}", hash.to_ascii_lowercase())
}

/// Percent-encode a relative path for use in a URL, keeping '/'.
fn encode_path(p: &str) -> String {
    let mut out = String::new();
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn parent_url(url: &str) -> &str {
    match url.rfind('/') {
        Some(i) => &url[..=i],
        None => url,
    }
}

/// Reject absolute paths and "..": the pack must never write outside the target folder.
fn safe_rel(p: &str) -> Result<PathBuf, SyncError> {
    let path = Path::new(p);
    if path.components().all(|c| matches!(c, Component::Normal(_))) && !p.is_empty() {
        Ok(path.to_path_buf())
    } else {
        err(format!("unsafe path in pack: {p}"))
    }
}

fn rel_dir(p: &str) -> &str {
    match p.rfind('/') {
        Some(i) => &p[..=i],
        None => "",
    }
}

// ---------- the sync ----------

pub struct Options {
    pub pack_url: String,
    pub dest: PathBuf,
    pub side: Side,
    pub workers: usize,
}

/// Background pre-download and Play may overlap; run syncs one after another.
static SYNC_LOCK: Mutex<()> = Mutex::new(());

pub fn sync(opts: &Options, tx: &Sender<Progress>) -> Result<Summary, SyncError> {
    let _one_at_a_time = SYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let agent = http_agent();
    let _ = tx.send(Progress::Checking);

    let pack: PackToml = toml::from_str(&get_text(&agent, &opts.pack_url)?)
        .map_err(|e| SyncError(format!("bad pack.toml: {e}")))?;
    let base = parent_url(&opts.pack_url).to_string();
    let index_path = safe_rel(&pack.index.file)?;
    let index_url = format!("{base}{}", encode_path(&pack.index.file));
    let index_raw = get_bytes(&agent, &index_url)?;
    if hash_hex(&pack.index.hash_format, &index_raw)? != pack.index.hash.to_ascii_lowercase() {
        return err("index.toml hash mismatch (pack is mid-upload? try again)");
    }
    let index: IndexToml = toml::from_str(
        std::str::from_utf8(&index_raw).map_err(|_| SyncError("index not utf-8".into()))?,
    )
    .map_err(|e| SyncError(format!("bad index.toml: {e}")))?;
    let index_base = format!(
        "{base}{}",
        encode_path(rel_dir(&index_path.to_string_lossy().replace('\\', "/")))
    );

    fs::create_dir_all(&opts.dest).map_err(|e| SyncError(e.to_string()))?;
    let manifest_path = opts.dest.join(MANIFEST_NAME);
    let old: Manifest = fs::read(&manifest_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();

    // Resolve every index entry to a target. Metafiles we already know are reused.
    let mut targets: Vec<Target> = Vec::new();
    let mut new_meta: BTreeMap<String, Target> = BTreeMap::new();
    let mut to_fetch: Vec<IndexEntry> = Vec::new();
    for e in &index.files {
        safe_rel(&e.file)?;
        let fmt = e.hash_format.clone().unwrap_or_else(|| index.hash_format.clone());
        if e.metafile {
            let key = tag(&fmt, &e.hash);
            match old.metafiles.get(&key) {
                Some(t) => {
                    new_meta.insert(key, t.clone());
                    targets.push(t.clone());
                }
                None => to_fetch.push(e.clone()),
            }
        } else {
            let local = e.alias.clone().unwrap_or_else(|| e.file.clone());
            safe_rel(&local)?;
            targets.push(Target {
                path: local.clone(),
                url: format!("{index_base}{}", encode_path(&e.file)),
                hash_format: fmt,
                hash: e.hash.to_ascii_lowercase(),
                name: local,
                side: None,
                preserve: e.preserve,
            });
        }
    }
    let fetched = parallel(to_fetch, opts.workers, |e: IndexEntry| -> Result<(String, Target), SyncError> {
        let fmt = e.hash_format.clone().unwrap_or_else(|| index.hash_format.clone());
        let raw = get_bytes(&agent, &format!("{index_base}{}", encode_path(&e.file)))?;
        if hash_hex(&fmt, &raw)? != e.hash.to_ascii_lowercase() {
            return err(format!("metafile hash mismatch: {}", e.file));
        }
        let m: MetaToml = toml::from_str(std::str::from_utf8(&raw).unwrap_or(""))
            .map_err(|er| SyncError(format!("bad metafile {}: {er}", e.file)))?;
        let url = match (m.download.url, m.download.mode.as_deref()) {
            (Some(u), _) => u,
            (None, Some(mode)) if mode.contains("curseforge") => {
                return err(format!(
                    "{} only allows downloads from the CurseForge site; add its Modrinth version to the pack",
                    m.name.unwrap_or(m.filename)
                ))
            }
            _ => return err(format!("no download url in {}", e.file)),
        };
        let path = format!("{}{}", rel_dir(&e.file), m.filename);
        safe_rel(&path)?;
        Ok((
            tag(&fmt, &e.hash),
            Target {
                path,
                url,
                hash_format: m.download.hash_format,
                hash: m.download.hash.to_ascii_lowercase(),
                name: m.name.unwrap_or(m.filename),
                side: m.side,
                preserve: e.preserve,
            },
        ))
    })?;
    for (k, t) in fetched {
        targets.push(t.clone());
        new_meta.insert(k, t);
    }

    // Keep only this side's files.
    targets.retain(|t| opts.side.wants(t.side.as_deref()));
    let wanted: HashSet<String> = targets.iter().map(|t| t.path.clone()).collect();

    // Decide what to download. Trust the manifest when the file is still there.
    let mut up_to_date = 0;
    let mut downloads = Vec::new();
    for t in &targets {
        let local = opts.dest.join(safe_rel(&t.path)?);
        let want = tag(&t.hash_format, &t.hash);
        let known = match (old.files.get(&t.path), fs::metadata(&local)) {
            (Some(i), Ok(m)) => i.hash == want && m.is_file() && m.len() == i.size,
            _ => false,
        };
        let exists_ok = known
            || (local.is_file()
                && (t.preserve || hash_file(&t.hash_format, &local).as_deref() == Some(&t.hash)));
        if exists_ok {
            up_to_date += 1;
        } else {
            downloads.push(t.clone());
        }
    }
    let stale: Vec<String> = old.files.keys().filter(|p| !wanted.contains(*p)).cloned().collect();
    let _ = tx.send(Progress::Plan {
        to_download: downloads.len(),
        up_to_date,
        to_remove: stale.len(),
    });

    let total = downloads.len();
    let done = Arc::new(Mutex::new(0usize));
    let dest = opts.dest.clone();
    let txc = tx.clone();
    parallel(downloads, opts.workers, |t: Target| -> Result<(), SyncError> {
        let data = get_bytes(&agent, &t.url)?;
        if hash_hex(&t.hash_format, &data)? != t.hash {
            return err(format!("{} failed its hash check", t.name));
        }
        let local = dest.join(safe_rel(&t.path)?);
        if let Some(dir) = local.parent() {
            fs::create_dir_all(dir).map_err(|e| SyncError(e.to_string()))?;
        }
        let part = local.with_extension("menma-part");
        {
            let mut f = fs::File::create(&part).map_err(|e| SyncError(e.to_string()))?;
            f.write_all(&data).map_err(|e| SyncError(e.to_string()))?;
        }
        fs::rename(&part, &local).map_err(|e| SyncError(format!("{}: {e}", t.path)))?;
        let n = {
            let mut d = done.lock().unwrap();
            *d += 1;
            *d
        };
        let _ = txc.send(Progress::FileDone { name: t.name.clone(), done: n, total });
        Ok(())
    })?;

    let mut removed = 0;
    for p in &stale {
        if let Ok(rel) = safe_rel(p)
            && fs::remove_file(opts.dest.join(rel)).is_ok() {
                removed += 1;
                let _ = tx.send(Progress::Removed { name: p.clone() });
            }
    }

    let manifest = Manifest {
        files: targets
            .iter()
            .filter_map(|t| {
                let size = fs::metadata(opts.dest.join(&t.path)).ok()?.len();
                Some((t.path.clone(), Installed { hash: tag(&t.hash_format, &t.hash), size }))
            })
            .collect(),
        metafiles: new_meta,
    };
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap())
        .map_err(|e| SyncError(e.to_string()))?;

    let summary = Summary {
        downloaded: total,
        up_to_date,
        removed,
        minecraft: pack.versions.get("minecraft").cloned().unwrap_or_default(),
        neoforge: pack.versions.get("neoforge").cloned(),
    };
    let _ = tx.send(Progress::Finished(summary.clone()));
    Ok(summary)
}

/// Run `f` over `items` on `workers` threads; first error wins, results keep no order.
fn parallel<T, R, F>(items: Vec<T>, workers: usize, f: F) -> Result<Vec<R>, SyncError>
where
    T: Send,
    R: Send,
    F: Fn(T) -> Result<R, SyncError> + Sync,
{
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let queue = Mutex::new(items);
    let results = Mutex::new(Vec::new());
    let failure: Mutex<Option<SyncError>> = Mutex::new(None);
    let n = workers.max(1).min(queue.lock().unwrap().len());
    thread::scope(|s| {
        for _ in 0..n {
            s.spawn(|| loop {
                if failure.lock().unwrap().is_some() {
                    return;
                }
                let next = queue.lock().unwrap().pop();
                let Some(item) = next else { return };
                match f(item) {
                    Ok(r) => results.lock().unwrap().push(r),
                    Err(e) => {
                        failure.lock().unwrap().get_or_insert(e);
                        return;
                    }
                }
            });
        }
    });
    match failure.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(results.into_inner().unwrap()),
    }
}

/// What the pack card shows: Minecraft version, NeoForge version, number of mods.
#[derive(Clone, Debug)]
pub struct PackInfo {
    pub name: String,
    pub minecraft: String,
    pub neoforge: Option<String>,
    pub mods: usize,
}

pub fn pack_info(pack_url: &str) -> Result<PackInfo, SyncError> {
    #[derive(Deserialize)]
    struct Head {
        name: Option<String>,
        index: IndexRef,
        versions: BTreeMap<String, String>,
    }
    let agent = http_agent();
    let pack: Head = toml::from_str(&get_text(&agent, pack_url)?)
        .map_err(|e| SyncError(format!("bad pack.toml: {e}")))?;
    let index_url = format!("{}{}", parent_url(pack_url), encode_path(&pack.index.file));
    let index: IndexToml = toml::from_str(&get_text(&agent, &index_url)?)
        .map_err(|e| SyncError(format!("bad index.toml: {e}")))?;
    let _ = (&pack.index.hash_format, &pack.index.hash);
    Ok(PackInfo {
        name: pack.name.unwrap_or_else(|| "Menma".into()),
        minecraft: pack.versions.get("minecraft").cloned().unwrap_or_default(),
        neoforge: pack.versions.get("neoforge").cloned(),
        mods: index.files.iter().filter(|f| f.metafile && f.file.starts_with("mods/")).count(),
    })
}

/// Convenience for the CLI: run a sync and print progress lines.
pub fn sync_print(opts: &Options) -> Result<Summary, SyncError> {
    let (tx, rx) = mpsc::channel();
    let printer = thread::spawn(move || {
        for p in rx {
            match p {
                Progress::Checking => println!("checking pack..."),
                Progress::Plan { to_download, up_to_date, to_remove } => println!(
                    "plan: {to_download} to download, {up_to_date} up to date, {to_remove} to remove"
                ),
                Progress::FileDone { name, done, total } => println!("({done}/{total}) {name}"),
                Progress::Removed { name } => println!("removed {name}"),
                Progress::Finished(_) => {}
            }
        }
    });
    let r = sync(opts, &tx);
    drop(tx);
    let _ = printer.join();
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escaping_paths() {
        assert!(safe_rel("mods/a.jar").is_ok());
        assert!(safe_rel("../evil.jar").is_err());
        assert!(safe_rel("C:/Windows/x").is_err());
        assert!(safe_rel("/abs").is_err());
        assert!(safe_rel("mods/../../x").is_err());
        assert!(safe_rel("").is_err());
    }

    #[test]
    fn encodes_paths_for_urls() {
        assert_eq!(encode_path("mods/a b+c.pw.toml"), "mods/a%20b%2Bc.pw.toml");
        assert_eq!(parent_url("https://x.y/a/b/pack.toml"), "https://x.y/a/b/");
        assert_eq!(rel_dir("mods/x.pw.toml"), "mods/");
        assert_eq!(rel_dir("x.toml"), "");
    }

    #[test]
    fn sides() {
        assert!(Side::Client.wants(Some("client")));
        assert!(!Side::Server.wants(Some("client")));
        assert!(Side::Server.wants(None));
        assert!(Side::Client.wants(Some("both")));
    }

    #[test]
    fn hashes() {
        assert_eq!(
            hash_hex("sha256", b"abc").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(hash_hex("sha1", b"abc").unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert!(hash_hex("md5", b"abc").is_err());
    }

    #[test]
    fn parallel_runs_all_and_reports_errors() {
        let r = parallel((0..50).collect(), 8, |i: i32| Ok::<_, SyncError>(i * 2)).unwrap();
        assert_eq!(r.iter().sum::<i32>(), (0..50).map(|i| i * 2).sum::<i32>());
        let e = parallel((0..50).collect(), 8, |i: i32| {
            if i == 7 { err("boom") } else { Ok(i) }
        });
        assert!(e.is_err());
    }
}
