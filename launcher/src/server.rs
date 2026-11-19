//! Minecraft server status ("server card") support: SRV resolution, Server List Ping,
//! and fetching 8x8 player skin faces from Mojang's session server.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde_json::Value;

const DEFAULT_PORT: u16 = 25565;
/// Protocol version sent in the handshake (1.21.x). Servers answer status for any value.
const PROTOCOL_VERSION: i32 = 767;
/// Upper bound for a single status packet (favicons can make it large).
const MAX_PACKET: usize = 8 * 1024 * 1024;

pub struct Player {
    pub name: String,
    pub uuid: String,
}

pub struct Status {
    pub ping_ms: u32,
    pub online: u32,
    pub max: u32,
    #[allow(dead_code)] // shown in a later version of the server card
    pub motd: String,
    #[allow(dead_code)]
    pub version: String,
    pub players: Vec<Player>,
}

// ---------------------------------------------------------------------------
// Addresses
// ---------------------------------------------------------------------------

/// Splits "host" or "host:port" (default port 25565). Handles bracketed IPv6 literals.
pub fn parse_address(addr: &str) -> (String, u16) {
    let addr = addr.trim();
    if let Some(rest) = addr.strip_prefix('[')
        && let Some((host, tail)) = rest.split_once(']')
    {
        let port = tail
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        return (host.to_string(), port);
    }
    // More than one ':' without brackets is a bare IPv6 address: no port.
    if addr.matches(':').count() == 1
        && let Some((host, port)) = addr.split_once(':')
    {
        return (host.to_string(), port.parse().unwrap_or(DEFAULT_PORT));
    }
    (addr.to_string(), DEFAULT_PORT)
}

fn has_explicit_port(addr: &str) -> bool {
    let addr = addr.trim();
    if let Some(rest) = addr.strip_prefix('[') {
        return rest
            .split_once(']')
            .is_some_and(|(_, t)| t.strip_prefix(':').is_some_and(|p| p.parse::<u16>().is_ok()));
    }
    addr.matches(':').count() == 1
        && addr
            .split_once(':')
            .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
}

/// Like [`parse_address`], but when no port is given looks up the SRV record
/// `_minecraft._tcp.<host>` and uses its target/port if present.
pub fn resolve(addr: &str) -> (String, u16) {
    let (host, port) = parse_address(addr);
    if has_explicit_port(addr) {
        return (host, port);
    }
    match srv_lookup(&format!("_minecraft._tcp.{host}")) {
        Some((target, p)) if !target.is_empty() && p != 0 => (target, p),
        _ => (host, DEFAULT_PORT),
    }
}

#[cfg(windows)]
fn srv_lookup(name: &str) -> Option<(String, u16)> {
    use windows_sys::Win32::NetworkManagement::Dns::{
        DNS_QUERY_STANDARD, DNS_RECORDA, DNS_TYPE_SRV, DnsFree, DnsQuery_W, DnsFreeRecordList,
    };

    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // DnsQuery_W fills the same layout as DNS_RECORDA; string fields are UTF-16 here.
    let mut records: *mut DNS_RECORDA = std::ptr::null_mut();
    // SAFETY: `wide` is NUL-terminated and outlives the call; `records` is a valid out
    // pointer. The returned list is only read while alive and is freed exactly once below.
    unsafe {
        let status = DnsQuery_W(
            wide.as_ptr(),
            DNS_TYPE_SRV,
            DNS_QUERY_STANDARD,
            std::ptr::null_mut(),
            &mut records,
            std::ptr::null_mut(),
        );
        if status != 0 || records.is_null() {
            return None;
        }
        let mut best: Option<(u16, String, u16)> = None; // (priority, target, port)
        let mut cur = records;
        while !cur.is_null() {
            let rec = &*cur;
            if rec.wType == DNS_TYPE_SRV {
                let srv = &rec.Data.SRV;
                let target = wide_ptr_to_string(srv.pNameTarget as *const u16);
                if best.as_ref().is_none_or(|b| srv.wPriority < b.0) {
                    best = Some((srv.wPriority, target, srv.wPort));
                }
            }
            cur = rec.pNext;
        }
        DnsFree(records as *const _, DnsFreeRecordList);
        best.map(|(_, t, p)| (t.trim_end_matches('.').to_string(), p))
    }
}

#[cfg(not(windows))]
fn srv_lookup(_name: &str) -> Option<(String, u16)> {
    None
}

/// # Safety
/// `p` must be null or point to a NUL-terminated UTF-16 string.
#[cfg(windows)]
unsafe fn wide_ptr_to_string(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    // SAFETY: guaranteed by caller contract.
    unsafe {
        while *p.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

// ---------------------------------------------------------------------------
// Protocol primitives
// ---------------------------------------------------------------------------

fn write_varint(out: &mut Vec<u8>, value: i32) {
    let mut v = value as u32;
    loop {
        if v & !0x7F == 0 {
            out.push(v as u8);
            return;
        }
        out.push((v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
}

fn read_varint<R: Read>(r: &mut R) -> Result<i32, String> {
    let mut result: u32 = 0;
    for i in 0..5 {
        let mut b = [0u8; 1];
        r.read_exact(&mut b).map_err(|e| format!("read: {e}"))?;
        result |= u32::from(b[0] & 0x7F) << (7 * i);
        if b[0] & 0x80 == 0 {
            return Ok(result as i32);
        }
    }
    Err("VarInt too long".into())
}

fn write_string(out: &mut Vec<u8>, s: &str) {
    write_varint(out, s.len() as i32);
    out.extend_from_slice(s.as_bytes());
}

/// Sends one length-prefixed packet (`id` + `body`).
fn send_packet<W: Write>(w: &mut W, id: i32, body: &[u8]) -> Result<(), String> {
    let mut payload = Vec::with_capacity(body.len() + 5);
    write_varint(&mut payload, id);
    payload.extend_from_slice(body);
    let mut frame = Vec::with_capacity(payload.len() + 5);
    write_varint(&mut frame, payload.len() as i32);
    frame.extend_from_slice(&payload);
    w.write_all(&frame)
        .and_then(|_| w.flush())
        .map_err(|e| format!("write: {e}"))
}

/// Reads one packet, returning `(id, body)`.
fn read_packet<R: Read>(r: &mut R) -> Result<(i32, Vec<u8>), String> {
    let len = read_varint(r)?;
    if len <= 0 || len as usize > MAX_PACKET {
        return Err(format!("bad packet length {len}"));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).map_err(|e| format!("read: {e}"))?;
    let mut cur = &buf[..];
    let id = read_varint(&mut cur)?;
    Ok((id, cur.to_vec()))
}

// ---------------------------------------------------------------------------
// Status parsing
// ---------------------------------------------------------------------------

/// Removes `§x` formatting codes (the section sign plus the following character).
fn strip_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

fn component_text(v: &Value, out: &mut String, depth: u8) {
    if depth > 16 {
        return;
    }
    match v {
        Value::String(s) => out.push_str(s),
        Value::Object(o) => {
            if let Some(Value::String(t)) = o.get("text") {
                out.push_str(t);
            }
            if let Some(Value::Array(extra)) = o.get("extra") {
                for e in extra {
                    component_text(e, out, depth + 1);
                }
            }
        }
        Value::Array(a) => {
            for e in a {
                component_text(e, out, depth + 1);
            }
        }
        _ => {}
    }
}

/// Converts a `description` (plain string or chat component) to plain text.
fn parse_description(v: &Value) -> String {
    let mut s = String::new();
    component_text(v, &mut s, 0);
    strip_formatting(&s).trim().to_string()
}

fn parse_status_json(json: &str, ping_ms: u32) -> Result<Status, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("bad status JSON: {e}"))?;
    let num = |x: Option<&Value>| {
        x.and_then(Value::as_u64).unwrap_or(0).min(u64::from(u32::MAX)) as u32
    };
    let players_obj = v.get("players");
    let players = players_obj
        .and_then(|p| p.get("sample"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let name = p.get("name")?.as_str()?;
                    let uuid = p.get("id").and_then(Value::as_str).unwrap_or("");
                    Some(Player {
                        name: strip_formatting(name),
                        uuid: uuid.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Status {
        ping_ms,
        online: num(players_obj.and_then(|p| p.get("online"))),
        max: num(players_obj.and_then(|p| p.get("max"))),
        motd: v.get("description").map(parse_description).unwrap_or_default(),
        version: v
            .get("version")
            .and_then(|x| x.get("name"))
            .and_then(Value::as_str)
            .map(strip_formatting)
            .unwrap_or_default(),
        players,
    })
}

// ---------------------------------------------------------------------------
// Ping
// ---------------------------------------------------------------------------

/// Performs a Server List Ping against `addr` (SRV-resolved when no port is given).
pub fn ping(addr: &str, timeout: Duration) -> Result<Status, String> {
    let (orig_host, _) = parse_address(addr);
    let (host, port) = resolve(addr);
    let sock = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("resolve {host}: {e}"))?
        .next()
        .ok_or_else(|| format!("no address for {host}"))?;
    let mut stream =
        TcpStream::connect_timeout(&sock, timeout).map_err(|e| format!("connect: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|_| stream.set_write_timeout(Some(timeout)))
        .map_err(|e| format!("socket: {e}"))?;
    let _ = stream.set_nodelay(true);

    // Handshake: protocol, server address (as the user typed it), port, next state = status.
    let mut hs = Vec::new();
    write_varint(&mut hs, PROTOCOL_VERSION);
    write_string(&mut hs, &orig_host);
    hs.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut hs, 1);
    send_packet(&mut stream, 0x00, &hs)?;
    send_packet(&mut stream, 0x00, &[])?; // status request

    let (id, body) = read_packet(&mut stream)?;
    if id != 0x00 {
        return Err(format!("unexpected packet id {id}"));
    }
    let mut cur = &body[..];
    let len = read_varint(&mut cur)?;
    if len < 0 || len as usize > cur.len() {
        return Err("truncated status string".into());
    }
    let json = String::from_utf8_lossy(&cur[..len as usize]).into_owned();

    // Latency: time a Ping/Pong round trip. Failure here is not fatal; the elapsed
    // time is still a usable estimate.
    let token: i64 = 0x4d45_4e4d_4100_0001;
    let started = Instant::now();
    let _ = send_packet(&mut stream, 0x01, &token.to_be_bytes())
        .and_then(|_| read_packet(&mut stream));
    let ping_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    parse_status_json(&json, ping_ms)
}

// ---------------------------------------------------------------------------
// Skin heads
// ---------------------------------------------------------------------------

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into()
}

fn b64_val(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// Minimal base64 decoder (standard and URL-safe alphabets, padding optional,
/// whitespace ignored).
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &c in input.as_bytes() {
        if c == b'=' {
            break;
        }
        if c.is_ascii_whitespace() {
            continue;
        }
        let v = b64_val(c).ok_or_else(|| format!("invalid base64 byte {c}"))?;
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

fn http_get_bytes(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?;
    resp.body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| format!("read {url}: {e}"))
}

/// Downloads the skin of `uuid` and returns its 8x8 face (with hat layer) as
/// `0xAARRGGBB` pixels, row-major.
pub fn fetch_head(uuid: &str) -> Result<[u32; 64], String> {
    let id: String = uuid.chars().filter(|&c| c != '-').collect();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid uuid {uuid:?}"));
    }
    let agent = http_agent();
    let profile_url = format!("https://sessionserver.mojang.com/session/minecraft/profile/{id}");
    let body = http_get_bytes(&agent, &profile_url, 1 << 20)?;
    let profile: Value =
        serde_json::from_slice(&body).map_err(|e| format!("bad profile JSON: {e}"))?;
    let encoded = profile
        .get("properties")
        .and_then(Value::as_array)
        .and_then(|props| {
            props
                .iter()
                .find(|p| p.get("name").and_then(Value::as_str) == Some("textures"))
        })
        .and_then(|p| p.get("value"))
        .and_then(Value::as_str)
        .ok_or("no textures property")?;
    let textures: Value = serde_json::from_slice(&base64_decode(encoded)?)
        .map_err(|e| format!("bad textures JSON: {e}"))?;
    let skin_url = textures
        .get("textures")
        .and_then(|t| t.get("SKIN"))
        .and_then(|s| s.get("url"))
        .and_then(Value::as_str)
        .ok_or("no skin url")?;
    // Mojang still hands out http:// texture URLs; the CDN serves https too.
    let skin_url = skin_url.replacen("http://", "https://", 1);
    let png_bytes = http_get_bytes(&agent, &skin_url, 4 << 20)?;
    let (rgba, w, h) = decode_png_rgba(&png_bytes)?;
    face_from_rgba(&rgba, w, h)
}

/// Decodes a PNG into tightly packed 8-bit RGBA.
fn decode_png_rgba(data: &[u8]) -> Result<(Vec<u8>, usize, usize), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    // Expand palettes/low bit depths and reduce 16-bit to 8-bit.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("png: {e}"))?;
    let size = reader.output_buffer_size().ok_or("png: image too large")?;
    let mut buf = vec![0u8; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("png: {e}"))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let src = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => src.to_vec(),
        png::ColorType::Rgb => src
            .as_chunks::<3>().0.iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => src
            .as_chunks::<2>().0.iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => src.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("png: unexpanded palette".into()),
    };
    if rgba.len() < w * h * 4 {
        return Err("png: short data".into());
    }
    Ok((rgba, w, h))
}

/// Crops the face (8..16, 8..16) and alpha-blends the hat layer (40..48, 8..16) over it.
/// `rgba` is 8-bit RGBA, `w` x `h` (64x64 or legacy 64x32).
fn face_from_rgba(rgba: &[u8], w: usize, h: usize) -> Result<[u32; 64], String> {
    if w < 48 || h < 16 || rgba.len() < w * h * 4 {
        return Err(format!("unexpected skin size {w}x{h}"));
    }
    let px = |x: usize, y: usize| -> [u8; 4] {
        let i = (y * w + x) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    };
    let mut face = [0u32; 64];
    for dy in 0..8 {
        for dx in 0..8 {
            let base = px(8 + dx, 8 + dy); // base layer is treated as opaque
            let hat = px(40 + dx, 8 + dy);
            let a = u32::from(hat[3]);
            let mix = |b: u8, t: u8| (u32::from(t) * a + u32::from(b) * (255 - a) + 127) / 255;
            let (r, g, b) = (mix(base[0], hat[0]), mix(base[1], hat[1]), mix(base[2], hat[2]));
            face[dy * 8 + dx] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
        }
    }
    Ok(face)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn varint_round_trip() {
        for v in [0, 1, 127, 128, 255, 25565, 2_097_151, i32::MAX, -1, -128, i32::MIN] {
            let mut buf = Vec::new();
            write_varint(&mut buf, v);
            assert!(buf.len() <= 5);
            assert_eq!(read_varint(&mut &buf[..]).unwrap(), v, "value {v}");
        }
        let mut buf = Vec::new();
        write_varint(&mut buf, -1);
        assert_eq!(buf, [0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
        let mut buf = Vec::new();
        write_varint(&mut buf, 300);
        assert_eq!(buf, [0xAC, 0x02]);
        assert!(read_varint(&mut &[0x80u8; 6][..]).is_err());
        assert!(read_varint(&mut &[0x80u8][..]).is_err());
    }

    #[test]
    fn description_parsing() {
        let v: Value = serde_json::from_str(r#""§aHello §lWorld""#).unwrap();
        assert_eq!(parse_description(&v), "Hello World");
        let v: Value = serde_json::from_str(
            r#"{"text":"A ","extra":[{"text":"§cB","color":"red"},{"text":" C","extra":[{"text":"!"}]}]}"#,
        )
        .unwrap();
        assert_eq!(parse_description(&v), "A B C!");
        let v: Value = serde_json::from_str("42").unwrap();
        assert_eq!(parse_description(&v), "");
    }

    #[test]
    fn address_parsing() {
        assert_eq!(parse_address("example.com"), ("example.com".into(), 25565));
        assert_eq!(parse_address("example.com:1234"), ("example.com".into(), 1234));
        assert_eq!(parse_address(" 1.2.3.4:5 "), ("1.2.3.4".into(), 5));
        assert_eq!(parse_address("[::1]:7"), ("::1".into(), 7));
        assert_eq!(parse_address("::1"), ("::1".into(), 25565));
        assert_eq!(parse_address("host:bad"), ("host".into(), 25565));
        // Explicit port skips SRV lookup entirely.
        assert_eq!(resolve("127.0.0.1:1234"), ("127.0.0.1".into(), 1234));
    }

    #[test]
    fn base64() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("TWFu").unwrap(), b"Man");
        assert_eq!(base64_decode("TWE=").unwrap(), b"Ma");
        assert_eq!(base64_decode("TQ==").unwrap(), b"M");
        assert_eq!(base64_decode("TQ").unwrap(), b"M");
        assert_eq!(base64_decode("+/8=").unwrap(), [0xFB, 0xFF]);
        assert_eq!(base64_decode("-_8").unwrap(), [0xFB, 0xFF]);
        assert!(base64_decode("T*Q=").is_err());
    }

    #[test]
    fn face_and_hat_crop() {
        let (w, h) = (64usize, 64usize);
        let mut img = vec![0u8; w * h * 4];
        let set = |img: &mut Vec<u8>, x: usize, y: usize, p: [u8; 4]| {
            let i = (y * w + x) * 4;
            img[i..i + 4].copy_from_slice(&p);
        };
        for y in 8..16 {
            for x in 8..16 {
                set(&mut img, x, y, [10, 20, 30, 255]);
            }
        }
        // Opaque hat pixel at face (1,0); half-transparent at (2,0); alpha 0 at (3,0).
        set(&mut img, 41, 8, [200, 100, 50, 255]);
        set(&mut img, 42, 8, [255, 255, 255, 128]);
        set(&mut img, 43, 8, [99, 99, 99, 0]);
        let face = face_from_rgba(&img, w, h).unwrap();
        assert_eq!(face[0], 0xFF0A141E);
        assert_eq!(face[1], 0xFFC86432);
        assert_eq!(face[2], 0xFF858A8F);
        assert_eq!(face[3], 0xFF0A141E);
        assert_eq!(face[63], 0xFF0A141E);
        assert!(face_from_rgba(&img, 32, 32).is_err());
        // Legacy 64x32 is accepted.
        assert!(face_from_rgba(&img[..64 * 32 * 4], 64, 32).is_ok());
    }

    #[test]
    fn png_decode_rgb_and_rgba() {
        let encode = |color, data: &[u8]| {
            let mut out = Vec::new();
            let mut enc = png::Encoder::new(&mut out, 64, 32);
            enc.set_color(color);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(data).unwrap();
            wr.finish().unwrap();
            out
        };
        let rgb = vec![7u8; 64 * 32 * 3];
        let (px, w, h) = decode_png_rgba(&encode(png::ColorType::Rgb, &rgb)).unwrap();
        assert_eq!((w, h, px.len()), (64, 32, 64 * 32 * 4));
        assert_eq!(&px[..4], &[7, 7, 7, 255]);
        let rgba = vec![9u8; 64 * 32 * 4];
        let (px, _, _) = decode_png_rgba(&encode(png::ColorType::Rgba, &rgba)).unwrap();
        assert_eq!(&px[..4], &[9, 9, 9, 9]);
        assert!(decode_png_rgba(b"not a png").is_err());
    }

    #[test]
    fn end_to_end_ping() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let (id, body) = read_packet(&mut s).unwrap();
            assert_eq!(id, 0x00);
            let mut cur = &body[..];
            assert_eq!(read_varint(&mut cur).unwrap(), PROTOCOL_VERSION);
            let n = read_varint(&mut cur).unwrap() as usize;
            assert_eq!(&cur[..n], b"127.0.0.1");
            assert_eq!(read_packet(&mut s).unwrap().0, 0x00); // status request
            let json = r##"{"version":{"name":"1.21.1","protocol":767},
                "players":{"max":20,"online":2,"sample":[
                    {"name":"Alice","id":"11111111-1111-1111-1111-111111111111"},
                    {"name":"Bob","id":"22222222-2222-2222-2222-222222222222"}]},
                "description":{"text":"§6Menma ","extra":[{"text":"Server"}]}}"##;
            let mut resp = Vec::new();
            write_string(&mut resp, json);
            send_packet(&mut s, 0x00, &resp).unwrap();
            let (id, body) = read_packet(&mut s).unwrap();
            assert_eq!(id, 0x01);
            send_packet(&mut s, 0x01, &body).unwrap();
        });
        let st = ping(&format!("127.0.0.1:{port}"), Duration::from_secs(5)).unwrap();
        server.join().unwrap();
        assert_eq!(st.online, 2);
        assert_eq!(st.max, 20);
        assert_eq!(st.motd, "Menma Server");
        assert_eq!(st.version, "1.21.1");
        assert_eq!(st.players.len(), 2);
        assert_eq!(st.players[0].name, "Alice");
        assert_eq!(st.players[1].uuid, "22222222-2222-2222-2222-222222222222");
        assert!(st.ping_ms < 5000);
    }

    #[test]
    fn status_without_sample() {
        let st = parse_status_json(r#"{"players":{"max":5,"online":0},"description":"x"}"#, 3)
            .unwrap();
        assert!(st.players.is_empty());
        assert_eq!((st.online, st.max, st.ping_ms), (0, 5, 3));
        assert!(parse_status_json("nope", 0).is_err());
    }

    #[test]
    #[ignore = "needs network"]
    fn fetch_notch_head() {
        let head = fetch_head("069a79f444e94726a5befca90e38aaf5").unwrap();
        assert!(head.iter().all(|p| p >> 24 == 0xFF));
    }
}
