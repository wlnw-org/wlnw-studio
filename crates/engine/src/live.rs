//! Live stations: detections read over the local network from a Wildlife Network Base or a BirdNET-Go station, so a
//! researcher can check them in Review like any other imported results. Only reads (HTTP GET) are made; nothing is sent
//! back to the station. Detections come from BirdNET-Go's API v2 (GET /api/v2/detections with start_date, end_date,
//! sortBy=date_asc, numResults and offset); a Base is recognised by GET /api/info on port 80 ("wildnetwork-base") and
//! runs BirdNET-Go on port 8080. BirdNET-Pi has no stable read API (its pages are HTML for today only), so it is refused
//! with a message.
//!
//! Each detection becomes one recording (its clip) in a deployment for the station, holding one detection under the model
//! id "import:live-station" that spans the whole clip. Clips are fetched from GET /api/v2/audio/<id> when first played in
//! Review and kept in "<project> clips/<station>/" next to the project file, so a clip once heard plays offline too.
//! Live recordings are never processed by Studio's models (run::plan gives them none).
use crate::store::Project;
use anyhow::{bail, Result};
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MODEL: &str = "import:live-station";
/// Deployments of live stations have a folder starting with this, followed by the station key.
pub const PREFIX: &str = "live:";

pub const UNREACHABLE: &str = "The station is not reachable. Studio works with a live station only while it is on the same network.";
pub const LOGIN: &str = "This station asks for a login to read detections or clips. Studio does not sign in to stations: in BirdNET-Go, turn off private mode (or allow detections and clips without a login), then try again.";
pub const PI: &str = "This is a BirdNET-Pi station. BirdNET-Pi has no stable read API, so Studio cannot import from it live. Copy its recordings and its detections export to this computer, then use Add a folder and Import results.";
pub const UNKNOWN: &str = "This address does not answer like a Wildlife Network Base or a BirdNET-Go station. Check the address (for example 192.168.1.20 or wnbase.local) and try again.";

pub const SCHEMA: &str = r#"
-- Recordings that are clips of a live station's detections: where to fetch the clip, and the station's own detection id
-- (so a detection is never added twice, whatever address the station had).
CREATE TABLE IF NOT EXISTS live_clips (file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE, station TEXT NOT NULL,
  remote_id TEXT NOT NULL, url TEXT NOT NULL, UNIQUE (station, remote_id));
"#;

/// Clip formats Studio cannot decode (BirdNET-Go can save Opus clips). AAC (.m4a, .aac) is decoded like WAV and FLAC.
const UNSUPPORTED: [&str; 2] = ["ogg", "opus"];
/// Labels BirdNET-Go can report that are not species. Human sounds are never imported.
const NOT_SPECIES: [&str; 12] = ["human vocal", "human non-vocal", "human whistle", "homo sapiens", "engine", "siren", "gun", "fireworks",
    "power tools", "environmental", "noise", "dog"];
const PAGE: usize = 1000;
const MAX_PAGES: usize = 200;

/// A station this project imports from, remembered in the project's settings under "live:<key>".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Station {
    /// "base:<hardware id>" for a Base, "bng:<host:port>" for BirdNET-Go.
    pub key: String,
    /// "base" or "birdnet-go".
    pub kind: String,
    pub name: String,
    /// The address as entered (normalised), used again by Refresh.
    pub address: String,
    /// BirdNET-Go's base URL (a Base's port 8080).
    pub api: String,
    pub lat: Option<f64>, pub lon: Option<f64>,
    /// Station-local date of the newest detection imported: Refresh starts there.
    #[serde(default)] pub last: Option<String>,
    #[serde(default)] pub imported_at: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct LiveReport {
    pub station: String, pub kind: String, pub key: String, pub from: String, pub to: String,
    /// Detections the station listed for the period.
    pub rows: usize,
    pub added: usize,
    /// Already in the project from an earlier import.
    pub already: usize,
    /// Detections without a clip (nothing to listen to), and non-species labels (noise, human, engine...).
    pub no_clip: usize, pub no_species: usize,
    pub model: String,
}

/// One detection as the station lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String, pub scientific: String, pub common: String, pub confidence: f64,
    /// Station-local time of the detection, and the station's offset from UTC when the station said.
    pub local: NaiveDateTime, pub tz_offset_min: Option<i32>,
    /// Clip file extension (from clipName); None when the detection has no clip.
    pub clip_ext: Option<String>,
}

/// "192.168.1.20", "wnbase.local:8080/", "http://x/y" -> "http://192.168.1.20", "http://wnbase.local:8080", "http://x".
pub fn normalize(addr: &str) -> Result<String> {
    let a = addr.trim();
    let rest = a.strip_prefix("http://").or_else(|| a.strip_prefix("HTTP://")).unwrap_or(a);
    if rest.starts_with("https://") { bail!("Use the station's local address (http), for example 192.168.1.20 or wnbase.local."); }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").trim();
    if host.is_empty() || host.contains(char::is_whitespace) || host.contains('@') { bail!("Enter the station's address, for example 192.168.1.20 or wnbase.local."); }
    Ok(format!("http://{}", host.to_lowercase()))
}

/// Host and port of a normalised URL ("http://x:8080" -> ("x", Some(8080))). IPv6 literals keep their brackets.
fn host_port(base: &str) -> (String, Option<u16>) {
    let h = base.trim_start_matches("http://");
    match h.rfind(':') {
        Some(i) if !h[i..].contains(']') => (h[..i].to_string(), h[i + 1..].parse().ok()),
        _ => (h.to_string(), None),
    }
}

/// The same host on another port (80 drops the port).
pub fn with_port(base: &str, port: u16) -> String {
    let (h, _) = host_port(base);
    if port == 80 { format!("http://{h}") } else { format!("http://{h}:{port}") }
}

/// A Base's GET /api/info: (hardware id, name, lat, lon), or None when it is not a Wildlife Network Base.
pub fn parse_info(json: &str) -> Option<(String, String, Option<f64>, Option<f64>)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if v.get("model")?.as_str()? != "wildnetwork-base" { return None; }
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty()).map(String::from);
    let id = s("hardwareId").or_else(|| s("hostname")).unwrap_or_else(|| "wnbase".into());
    let name = s("name").or_else(|| s("hostname")).unwrap_or_else(|| "Wildlife Network Base".into());
    let f = |k: &str| v.get(k).and_then(|x| x.as_f64()).filter(|x| x.is_finite());
    let (lat, lon) = (f("latitude"), f("longitude"));
    // 0,0 is a Base whose place was never set.
    let (lat, lon) = if lat == Some(0.0) && lon == Some(0.0) { (None, None) } else { (lat, lon) };
    Some((id, name, lat, lon))
}

/// True for BirdNET-Pi's web page (a station Studio cannot read live).
pub fn is_birdnet_pi(html: &str) -> bool { html.contains("BirdNET-Pi") }

/// One page of GET /api/v2/detections: the rows and the total the station has for the query.
pub fn parse_page(json: &str) -> Result<(Vec<Row>, usize)> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|_| anyhow::anyhow!("{UNKNOWN}"))?;
    let Some(data) = v.get("data").and_then(|d| d.as_array()) else { bail!("{UNKNOWN}") };
    let total = v.get("total").and_then(|t| t.as_u64()).unwrap_or(data.len() as u64) as usize;
    let mut rows = Vec::with_capacity(data.len());
    for d in data {
        let id = match d.get("id") { Some(serde_json::Value::Number(n)) => n.to_string(), Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(), _ => continue };
        let s = |k: &str| d.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
        let (local, tz) = match DateTime::parse_from_rfc3339(&s("timestamp")) {
            Ok(t) => (t.naive_local(), Some(t.offset().local_minus_utc() / 60)),
            Err(_) => match NaiveDateTime::parse_from_str(&format!("{} {}", s("date"), s("time")), "%Y-%m-%d %H:%M:%S") {
                Ok(t) => (t, None),
                Err(_) => continue,
            },
        };
        let clip = s("clipName");
        let clip_ext = (!clip.is_empty()).then(|| Path::new(&clip).extension().and_then(|e| e.to_str()).unwrap_or("wav").to_ascii_lowercase());
        rows.push(Row { id, scientific: s("scientificName"), common: s("commonName"), confidence: d.get("confidence").and_then(|c| c.as_f64()).unwrap_or(0.0).clamp(0.0, 1.0),
            local, tz_offset_min: tz, clip_ext });
    }
    Ok((rows, total))
}

/// A scientific name worth keeping: two words, genus capitalised, not one of BirdNET's non-species labels.
pub fn is_species(scientific: &str, common: &str) -> bool {
    let n = scientific.trim();
    if NOT_SPECIES.iter().any(|x| n.eq_ignore_ascii_case(x) || common.trim().eq_ignore_ascii_case(x)) { return false; }
    let mut w = n.split_whitespace();
    matches!((w.next(), w.next()), (Some(g), Some(e)) if g.chars().next().is_some_and(char::is_uppercase) && e.chars().all(|c| c.is_lowercase() || c == '-'))
}

/// Folder where a station's clips are kept: "<project stem> clips/<station key>" next to the project file.
pub fn clip_dir(project: &Path, key: &str) -> PathBuf {
    let stem = project.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "project".into());
    let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '-' }).collect();
    project.with_file_name(format!("{stem} clips")).join(safe)
}

/// Adds the listed detections to the project (one recording and one detection each). Detections already imported, and
/// ones without a clip or a species, are counted and skipped. Updates `station.last`.
pub fn store(p: &Project, station: &mut Station, rows: &[Row], rep: &mut LiveReport) -> Result<()> {
    let folder = format!("{PREFIX}{}", station.key);
    let tz = rows.iter().find_map(|r| r.tz_offset_min);
    let kind = if station.kind == "base" { "Wildlife Network Base" } else { "BirdNET-Go" };
    let tx = p.db.unchecked_transaction()?;
    // A new station takes the place the Base reports; one the user already placed keeps theirs.
    tx.execute("INSERT INTO deployments (name, folder, serial, model, lat, lon, tz_offset_min, tz_known) VALUES (?1, ?2, '', ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT (folder, serial) DO UPDATE SET lat = COALESCE(deployments.lat, excluded.lat), lon = COALESCE(deployments.lon, excluded.lon),
          tz_offset_min = CASE WHEN deployments.tz_known = 1 THEN deployments.tz_offset_min ELSE excluded.tz_offset_min END, tz_known = MAX(deployments.tz_known, excluded.tz_known)",
        params![station.name, folder, kind, station.lat, station.lon, tz.unwrap_or(0), tz.is_some() as i64])?;
    let dep: i64 = tx.query_row("SELECT id FROM deployments WHERE folder = ?1 AND serial = ''", [&folder], |r| r.get(0))?;
    let dir = clip_dir(&p.path, &station.key);
    let mut newest: Option<NaiveDate> = station.last.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    for r in rows {
        rep.rows += 1;
        newest = newest.max(Some(r.local.date()));
        let Some(ext) = &r.clip_ext else { rep.no_clip += 1; continue };
        if !is_species(&r.scientific, &r.common) { rep.no_species += 1; continue; }
        let seen: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM live_clips WHERE station = ?1 AND remote_id = ?2)", params![station.key, r.id], |x| x.get(0))?;
        if seen { rep.already += 1; continue; }
        let path = dir.join(format!("{}.{ext}", r.id)).to_string_lossy().to_string();
        let file: i64 = tx.query_row("INSERT INTO files (deployment_id, path, start_local, time_source) VALUES (?1, ?2, ?3, 'live station')
            ON CONFLICT (path) DO UPDATE SET deployment_id = excluded.deployment_id RETURNING id",
            params![dep, path, r.local.format("%Y-%m-%d %H:%M:%S").to_string()], |x| x.get(0))?;
        tx.execute("INSERT INTO live_clips (file_id, station, remote_id, url) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (file_id) DO NOTHING",
            params![file, station.key, r.id, format!("{}/api/v2/audio/{}", station.api, r.id)])?;
        let sid: i64 = tx.query_row("INSERT INTO species (scientific, common) VALUES (?1, ?2)
            ON CONFLICT (scientific) DO UPDATE SET common = CASE WHEN species.common = '' THEN excluded.common ELSE species.common END RETURNING id",
            params![r.scientific, r.common], |x| x.get(0))?;
        // The whole clip is the detection (BirdNET-Go keeps a few seconds before it); end_s becomes the clip's length once fetched.
        tx.execute("INSERT INTO detections (file_id, model, start_s, end_s, species, confidence, in_range) VALUES (?1, ?2, 0, 3, ?3, ?4, 1)",
            params![file, MODEL, sid, r.confidence])?;
        tx.execute("INSERT INTO file_models (file_id, model, status, error, processed_at) VALUES (?1, ?2, 'done', NULL, datetime('now'))
            ON CONFLICT (file_id, model) DO NOTHING", params![file, MODEL])?;
        rep.added += 1;
    }
    tx.commit()?;
    station.last = newest.map(|d| d.format("%Y-%m-%d").to_string());
    rep.model = MODEL.into();
    Ok(())
}

/// Stations this project has imported from, most recent first.
pub fn stations(p: &Project) -> Result<Vec<Station>> {
    let mut st = p.db.prepare("SELECT value FROM settings WHERE key LIKE 'live:%'")?;
    let mut out: Vec<Station> = st.query_map([], |r| r.get::<_, String>(0))?.filter_map(|v| v.ok().and_then(|v| serde_json::from_str(&v).ok())).collect();
    out.sort_by(|a, b| b.imported_at.cmp(&a.imported_at));
    Ok(out)
}

fn save(p: &Project, s: &Station) -> Result<()> { p.set_setting(&format!("{PREFIX}{}", s.key), &serde_json::to_string(s)?) }

// ---- Network (GET only) ----

fn agent(secs: u64) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(5)).timeout(Duration::from_secs(secs))
        .user_agent(concat!("WildlifeNetworkStudio/", env!("CARGO_PKG_VERSION"))).build()
}

enum Got { Ok(ureq::Response), Status(u16), Unreachable }
fn get(a: &ureq::Agent, url: &str) -> Got {
    match a.get(url).call() { Ok(r) => Got::Ok(r), Err(ureq::Error::Status(c, _)) => Got::Status(c), Err(ureq::Error::Transport(_)) => Got::Unreachable }
}
fn text(r: ureq::Response) -> String { r.into_string().unwrap_or_default() }

/// What answers at an address: a Base (and its BirdNET-Go) or a BirdNET-Go station.
pub fn identify(addr: &str) -> Result<Station> {
    let base = normalize(addr)?;
    let a = agent(10);
    let (host, port) = host_port(&base);
    let info_url = if port == Some(8080) { with_port(&base, 80) } else { base.clone() };
    let mut reached = false;
    let mut info = None;
    match get(&a, &format!("{info_url}/api/info")) {
        Got::Ok(r) => { reached = true; info = parse_info(&text(r)); }
        Got::Status(_) => reached = true,
        Got::Unreachable => {}
    }
    if let Some((id, name, lat, lon)) = info {
        let api = if port.is_some_and(|p| p != 80) { base.clone() } else { with_port(&base, 8080) };
        return Ok(Station { key: format!("base:{id}"), kind: "base".into(), name, address: base, api, lat, lon, last: None, imported_at: None });
    }
    match get(&a, &format!("{base}/api/v2/ping")) {
        Got::Ok(_) => {
            let name = match get(&a, &format!("{base}/api/v2/settings/dashboard")) {
                Got::Ok(r) => serde_json::from_str::<serde_json::Value>(&text(r)).ok().and_then(|v| v.get("node_name").and_then(|n| n.as_str()).map(String::from)),
                _ => None,
            }.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| host.clone());
            let key = format!("bng:{}:{}", host, port.unwrap_or(80));
            return Ok(Station { key, kind: "birdnet-go".into(), name, address: base.clone(), api: base, lat: None, lon: None, last: None, imported_at: None });
        }
        Got::Status(401 | 403) => bail!("{LOGIN}"),
        Got::Status(_) => reached = true,
        Got::Unreachable => {}
    }
    // An address with no port may be a Base whose wnbase API did not answer: try its BirdNET-Go.
    if port.is_none() {
        if let Got::Ok(_) = get(&a, &format!("{}/api/v2/ping", with_port(&base, 8080))) {
            return identify(&with_port(&base, 8080));
        }
    }
    if !reached { bail!("{UNREACHABLE}"); }
    if let Got::Ok(r) = get(&a, &format!("{base}/")) { if is_birdnet_pi(&text(r)) { bail!("{PI}"); } }
    bail!("{UNKNOWN}")
}

/// Station-local dates as YYYY-MM-DD, checked.
fn date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|_| anyhow::anyhow!("Dates must look like 2026-05-31."))
}

/// Imports a station's detections from `from` to `to` (station-local dates, inclusive).
pub fn import(p: &Project, addr: &str, from: &str, to: &str) -> Result<LiveReport> {
    let mut st = identify(addr)?;
    // The same station imported before (a Base keeps its key when its address changes) keeps its cursor and place.
    if let Some(old) = stations(p)?.into_iter().find(|s| s.key == st.key) { st.last = old.last; }
    pull(p, st, from, to)
}

/// Pulls new detections from a station imported before: from the day of the newest one imported until today.
pub fn refresh(p: &Project, key: &str) -> Result<LiveReport> {
    let Some(old) = stations(p)?.into_iter().find(|s| s.key == key) else { bail!("This station is not in the project.") };
    let mut st = identify(&old.address)?;
    if st.key != old.key { bail!("Another station now answers at {}. Add it with its own address.", old.address); }
    st.last = old.last.clone();
    let today = chrono::Local::now().date_naive();
    // One day past today: the station's calendar may be ahead of this computer's.
    let from = old.last.unwrap_or_else(|| today.format("%Y-%m-%d").to_string());
    pull(p, st, &from, &(today + chrono::Duration::days(1)).format("%Y-%m-%d").to_string())
}

fn pull(p: &Project, mut st: Station, from: &str, to: &str) -> Result<LiveReport> {
    let (f, t) = (date(from)?, date(to)?);
    if f > t { bail!("The first day must not be after the last day."); }
    let mut rep = LiveReport { station: st.name.clone(), kind: st.kind.clone(), key: st.key.clone(), from: f.to_string(), to: t.to_string(), ..Default::default() };
    let a = agent(60);
    let mut offset = 0;
    for _ in 0..MAX_PAGES {
        let url = format!("{}/api/v2/detections?start_date={f}&end_date={t}&sortBy=date_asc&numResults={PAGE}&offset={offset}", st.api);
        let body = match get(&a, &url) {
            Got::Ok(r) => text(r),
            Got::Status(401 | 403) => bail!("{LOGIN}"),
            Got::Status(c) => bail!("The station answered {c} when asked for its detections. It may run an older BirdNET-Go: update it, then try again."),
            Got::Unreachable => bail!("{UNREACHABLE}"),
        };
        let (rows, total) = parse_page(&body)?;
        store(p, &mut st, &rows, &mut rep)?;
        offset += rows.len();
        if rows.is_empty() || offset >= total { break; }
    }
    st.imported_at = Some(chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string());
    save(p, &st)?;
    rep.model = MODEL.into();
    Ok(rep)
}

/// For a live station's clip: fetches it into the project folder when it is not there yet, sets the recording's length and
/// widens its detection to the whole clip. Returns the clip's length, or None for an ordinary recording.
pub fn ensure_clip(p: &Project, path: &str) -> Result<Option<f64>> {
    let Some((file, url, dur)): Option<(i64, String, Option<f64>)> = p.db.query_row(
        "SELECT l.file_id, l.url, f.duration_s FROM live_clips l JOIN files f ON f.id = l.file_id WHERE f.path = ?1", [path],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()? else { return Ok(None) };
    let local = Path::new(path);
    let ext = local.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if UNSUPPORTED.contains(&ext.as_str()) {
        bail!("This station saves clips as Opus ({ext}), which Studio cannot play yet. In BirdNET-Go's audio export settings choose WAV, FLAC or AAC; clips saved after that can be reviewed here.");
    }
    if let (true, Some(d)) = (local.exists(), dur) { return Ok(Some(d)); }
    if !local.exists() {
        let r = match get(&agent(60), &url) {
            Got::Ok(r) => r,
            Got::Status(401 | 403) => bail!("{LOGIN}"),
            Got::Status(404) => bail!("The station no longer has this clip (it may have been deleted to save space)."),
            Got::Status(c) => bail!("The station answered {c} when asked for this clip."),
            Got::Unreachable => bail!("{UNREACHABLE}"),
        };
        let mut bytes = Vec::new();
        r.into_reader().take(200 << 20).read_to_end(&mut bytes).map_err(|_| anyhow::anyhow!("{UNREACHABLE}"))?;
        if let Some(dir) = local.parent() { std::fs::create_dir_all(dir)?; }
        let tmp = local.with_extension("part");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, local)?;
        p.db.execute("UPDATE files SET bytes = ?2 WHERE id = ?1", params![file, bytes.len() as i64])?;
    }
    let secs = match crate::audio::duration(local) {
        Some(s) => s,
        // Clips are short (BirdNET-Go keeps up to a couple of minutes), so decoding one to measure it is cheap.
        None => crate::audio::read_segment(local, 0.0, 180.0).map_err(|_| { std::fs::remove_file(local).ok(); anyhow::anyhow!("The station's clip could not be read as audio. Try again in a minute: a clip is saved a few seconds after its detection.") })?.len() as f64 / crate::SR as f64,
    };
    p.db.execute("UPDATE files SET duration_s = ?2, sample_rate = COALESCE(sample_rate, ?3) WHERE id = ?1", params![file, secs, crate::audio::probe_rate(local).map(|r| r as i64)])?;
    p.db.execute("UPDATE detections SET end_s = ?2 WHERE file_id = ?1 AND model = ?3", params![file, secs.max(0.1), MODEL])?;
    Ok(Some(secs))
}

/// One line for History.
pub fn summary(x: &LiveReport) -> String {
    use crate::history::n;
    format!("Imported {} detections from the live station {} ({} to {}; {} listed, {} already in the project)", n(x.added as i64), x.station, x.from, x.to,
        n(x.rows as i64), n(x.already as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE_JSON: &str = r#"{"data":[
      {"id":101,"date":"2026-11-18","time":"06:12:03","timestamp":"2026-11-18T06:12:03+01:00","beginTime":"2026-11-18T06:12:03+01:00","endTime":"2026-11-18T06:12:06+01:00",
       "speciesCode":"eurrob1","scientificName":"Erithacus rubecula","commonName":"European Robin","confidence":0.91,"clipName":"erithacus_rubecula_91p_20261118T061203Z.wav","verified":"","locked":false},
      {"id":102,"date":"2026-11-18","time":"06:13:00","timestamp":"2026-11-18T06:13:00+01:00","scientificName":"Turdus merula","commonName":"Eurasian Blackbird","confidence":0.75,"clipName":"turdus.flac"},
      {"id":103,"date":"2026-11-18","time":"06:14:00","scientificName":"Human vocal","commonName":"Human vocal","confidence":0.9,"clipName":"h.wav"},
      {"id":104,"date":"2026-11-18","time":"06:15:00","scientificName":"Parus major","commonName":"Great Tit","confidence":0.8,"clipName":""},
      {"id":105,"date":"bad","time":"x","scientificName":"Parus major","commonName":"Great Tit","confidence":0.8}
    ],"total":5,"limit":1000,"offset":0,"current_page":1,"total_pages":1}"#;

    #[test]
    fn normalises_addresses() {
        assert_eq!(normalize(" 192.168.1.20 ").unwrap(), "http://192.168.1.20");
        assert_eq!(normalize("http://WNBase.local:8080/ui/dashboard").unwrap(), "http://wnbase.local:8080");
        assert!(normalize("https://x").is_err());
        assert!(normalize("").is_err());
        assert_eq!(with_port("http://wnbase.local", 8080), "http://wnbase.local:8080");
        assert_eq!(with_port("http://wnbase.local:8080", 80), "http://wnbase.local");
        assert_eq!(host_port("http://[fe80::1]:8080"), ("[fe80::1]".to_string(), Some(8080)));
        assert_eq!(host_port("http://[fe80::1]"), ("[fe80::1]".to_string(), None));
    }

    #[test]
    fn reads_base_info() {
        let j = r#"{"hardwareId":"10000000abcd1234","model":"wildnetwork-base","hostname":"wnbase","configured":true,"name":"Hill top","latitude":51.5,"longitude":-0.12}"#;
        assert_eq!(parse_info(j), Some(("10000000abcd1234".into(), "Hill top".into(), Some(51.5), Some(-0.12))));
        let unset = r#"{"hardwareId":"a","model":"wildnetwork-base","hostname":"wnbase","name":null,"latitude":0,"longitude":0}"#;
        assert_eq!(parse_info(unset), Some(("a".into(), "wnbase".into(), None, None)));
        assert_eq!(parse_info(r#"{"model":"other"}"#), None);
        assert_eq!(parse_info("<html>"), None);
        assert!(is_birdnet_pi("<title>BirdNET-Pi</title>"));
    }

    #[test]
    fn reads_detection_pages() {
        let (rows, total) = parse_page(PAGE_JSON).unwrap();
        assert_eq!((rows.len(), total), (4, 5));
        assert_eq!(rows[0].id, "101");
        assert_eq!(rows[0].tz_offset_min, Some(60));
        assert_eq!(rows[0].local.to_string(), "2026-11-18 06:12:03");
        assert_eq!(rows[0].clip_ext.as_deref(), Some("wav"));
        assert_eq!(rows[1].clip_ext.as_deref(), Some("flac"));
        assert_eq!(rows[2].tz_offset_min, None);
        assert_eq!(rows[3].clip_ext, None);
        assert!(parse_page("<html></html>").is_err());
        assert!(is_species("Erithacus rubecula", "European Robin"));
        assert!(!is_species("Human vocal", "Human vocal"));
        assert!(!is_species("Engine", "Engine"));
        assert!(!is_species("Canis familiaris", "Dog"));
    }

    #[test]
    fn stores_once_and_behaves_like_imports() {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(&dir.path().join("Field.wlnw")).unwrap();
        let mut st = Station { key: "base:abc".into(), kind: "base".into(), name: "Hill top".into(), address: "http://10.0.0.5".into(), api: "http://10.0.0.5:8080".into(),
            lat: Some(51.5), lon: Some(-0.12), last: None, imported_at: None };
        let (rows, _) = parse_page(PAGE_JSON).unwrap();
        let mut rep = LiveReport::default();
        store(&p, &mut st, &rows, &mut rep).unwrap();
        assert_eq!((rep.rows, rep.added, rep.already, rep.no_clip, rep.no_species), (4, 2, 0, 1, 1));
        assert_eq!(st.last.as_deref(), Some("2026-11-18"));
        // A second import (Refresh) adds nothing twice.
        let mut again = LiveReport::default();
        store(&p, &mut st, &rows, &mut again).unwrap();
        assert_eq!((again.added, again.already), (0, 2));
        let deps = p.deployments().unwrap();
        assert_eq!(deps.len(), 1);
        let d = &deps[0];
        assert_eq!((d.name.as_str(), d.folder.as_str(), d.lat, d.tz_offset_min, d.tz_known), ("Hill top", "live:base:abc", Some(51.5), 60, true));
        let (url, path): (String, String) = p.db.query_row("SELECT l.url, f.path FROM live_clips l JOIN files f ON f.id = l.file_id WHERE l.remote_id = '101'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(url, "http://10.0.0.5:8080/api/v2/audio/101");
        assert!(path.ends_with("101.wav") && path.contains("Field clips"));
        // Times are the station's, in UTC for exports; Studio's models never process live clips.
        let m: &dyn rusqlite::ToSql = &MODEL;
        let det = p.detections("d.model = ?1", &[m], 10).unwrap();
        assert_eq!(det.len(), 2);
        assert_eq!(det[0].utc.unwrap().to_string(), "2026-11-18 05:12:03");
        assert_eq!(det[0].site, "Hill top");
        let plan = crate::run::plan(&p, &["birdnet-2.4".to_string()]).unwrap();
        assert!(plan[&d.id].is_empty());
        // Reviewed like any imported result, and accepted for export and publishing.
        crate::review::set_verdict(&p, det[1].id, Some("yes")).unwrap();
        assert!(crate::review::species(&p).unwrap().iter().any(|s| s.model == MODEL && s.scientific == "Turdus merula" && s.reviewed == 1));
        assert!(p.accepted().unwrap().iter().any(|a| a.model == MODEL && a.scientific == "Turdus merula"));
        // An ordinary recording is not a live clip.
        assert_eq!(ensure_clip(&p, "/somewhere/else.wav").unwrap(), None);
        save(&p, &st).unwrap();
        assert_eq!(stations(&p).unwrap(), vec![st]);
    }

    #[test]
    fn unsupported_clips_say_why() {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(&dir.path().join("P.wlnw")).unwrap();
        let mut st = Station { key: "bng:x:8080".into(), kind: "birdnet-go".into(), name: "x".into(), address: "http://x:8080".into(), api: "http://x:8080".into(),
            lat: None, lon: None, last: None, imported_at: None };
        let mut r = parse_page(PAGE_JSON).unwrap().0;
        r[0].clip_ext = Some("opus".into());
        store(&p, &mut st, &r[..1], &mut LiveReport::default()).unwrap();
        let path: String = p.db.query_row("SELECT path FROM files", [], |x| x.get(0)).unwrap();
        assert!(ensure_clip(&p, &path).unwrap_err().to_string().contains("cannot play"));
    }

    #[test]
    fn base_m4a_clips_play() {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(&dir.path().join("P.wlnw")).unwrap();
        let mut st = Station { key: "base:m4a".into(), kind: "base".into(), name: "Base".into(), address: "http://x".into(), api: "http://x:8080".into(),
            lat: None, lon: None, last: None, imported_at: None };
        let json = PAGE_JSON.replace("erithacus_rubecula_91p_20261118T061203Z.wav", "erithacus_rubecula_91p_20261118T061203Z.m4a");
        let r = parse_page(&json).unwrap().0;
        assert_eq!(r[0].clip_ext.as_deref(), Some("m4a"));
        store(&p, &mut st, &r[..1], &mut LiveReport::default()).unwrap();
        let path: String = p.db.query_row("SELECT path FROM files", [], |x| x.get(0)).unwrap();
        assert!(path.ends_with("101.m4a"));
        // As if already fetched from the Base: measured, detection widened to the whole clip, and Review's clip decodes.
        std::fs::create_dir_all(Path::new(&path).parent().unwrap()).unwrap();
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/tone.m4a"), &path).unwrap();
        let secs = ensure_clip(&p, &path).unwrap().unwrap();
        assert!((secs - 1.5).abs() < 0.1, "{secs}");
        let (rate, end): (i64, f64) = p.db.query_row("SELECT f.sample_rate, d.end_s FROM files f JOIN detections d ON d.file_id = f.id", [], |x| Ok((x.get(0)?, x.get(1)?))).unwrap();
        assert_eq!((rate, end), (48_000, secs));
        let c = crate::review::clip_for(Path::new(&path), 0.0, secs, Some(MODEL)).unwrap();
        assert!(c.width > 0 && c.height > 0 && c.wav.len() > 44 && (c.total_s - secs).abs() < 0.05);
    }
}
