//! "Wildlife Network when online": public, read-only facts from https://wlnw.org about the place a recorder stands in
//! (species known nearby from GBIF, usual arrival weeks, protected areas) and about species (photo, one line).
//! Only a recorder's position rounded to 0.1 degree (about 10 km) and species names are ever sent; never audio, files
//! or results. Every answer is kept in the project's web_cache table for 30 days, so it works offline later and travels
//! with the project. Network access is passed in as a function, so tests run without a network.
use crate::store::Project;
use anyhow::{bail, Result};
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};

pub const SITE: &str = "https://wlnw.org";
/// Cached answers older than this are fetched again (and still used when that fails).
pub const MAX_AGE_DAYS: i64 = 30;
/// Radius of the "known nearby" checklist.
pub const KM: u32 = 25;
pub const GROUPS: [&str; 4] = ["avian", "bat", "amphibian", "insect"];
/// Arrivals below this share of redraws finding a season are not shown (same rule as the site and BirdEcho).
pub const MIN_ARRIVAL_SUPPORT: f64 = 0.5;
/// A bird missing from a nearby list this long is worth a careful look; shorter lists say too little.
const MIN_LIST_FOR_WARNING: usize = 30;
const USER_AGENT: &str = concat!("WildlifeNetworkStudio/", env!("CARGO_PKG_VERSION"));

/// Fetches a URL and returns its JSON. `None` means offline: answer from the cache only.
pub type Net<'a> = Option<&'a dyn Fn(&str) -> Result<Value>>;

// ---------- rounding, names, URLs ----------

/// Rounds to 0.1 degree, the precision the site works at (about 11 km north to south).
pub fn round1(x: f64) -> f64 { let r = (x * 10.0).round() / 10.0; if r == 0.0 { 0.0 } else { r } }
/// A rounded coordinate as sent and as part of cache keys ("-0.0" written as "0.0").
pub fn fmt1(x: f64) -> String { format!("{:.1}", round1(x)) }

/// Lower case, single spaces.
pub fn norm(name: &str) -> String { name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase() }
/// Genus and species only (drops a subspecies or anything after the second word).
pub fn binomial(name: &str) -> String { norm(name).split(' ').take(2).collect::<Vec<_>>().join(" ") }

/// Species page on wlnw.org: scientific name lowercased, spaces to hyphens.
pub fn species_url(scientific: &str) -> String { format!("{SITE}/species/{}", norm(scientific).replace(' ', "-")) }

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub fn checklist_url(lat: f64, lon: f64, group: &str) -> String {
    format!("{SITE}/api/v1/checklist?lat={}&lon={}&km={KM}&group={}", fmt1(lat), fmt1(lon), enc(group))
}
pub fn arrivals_url(lat: f64, lon: f64) -> String { format!("{SITE}/api/v1/arrivals?lat={}&lon={}", fmt1(lat), fmt1(lon)) }
pub fn areas_url(lat: f64, lon: f64) -> String { format!("{SITE}/api/v1/areas/nearby?lat={}&lon={}&km=50", fmt1(lat), fmt1(lon)) }
pub fn media_url(names: &[String]) -> String { format!("{SITE}/api/v1/media?names={}", enc(&names.join(","))) }

// ---------- network ----------

/// After a failed request the network is left alone for a few minutes, so an offline laptop never waits on timeouts.
static OFFLINE_UNTIL: AtomicI64 = AtomicI64::new(0);

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(60)).user_agent(USER_AGENT).build()
}

/// GET a wlnw.org API URL as JSON (the real network; tests pass their own function instead).
pub fn http_json(url: &str) -> Result<Value> {
    if !url.starts_with(SITE) { bail!("only wlnw.org is asked"); }
    if Utc::now().timestamp() < OFFLINE_UNTIL.load(Ordering::Relaxed) { bail!("offline"); }
    match agent().get(url).call() {
        Ok(r) => Ok(r.into_json()?),
        Err(ureq::Error::Status(code, _)) => bail!("Wildlife Network said {code}"),
        Err(e) => { OFFLINE_UNTIL.store(Utc::now().timestamp() + 300, Ordering::Relaxed); bail!("offline: {e}") }
    }
}

/// A species thumbnail (from the media answer) as a data: URL, so the app's content policy can stay closed to other hosts.
fn http_image(url: &str) -> Result<Value> {
    if !url.starts_with("https://") { bail!("not https"); }
    if Utc::now().timestamp() < OFFLINE_UNTIL.load(Ordering::Relaxed) { bail!("offline"); }
    let r = agent().get(url).call()?;
    let ty = r.content_type().to_string();
    if !ty.starts_with("image/") { bail!("not an image"); }
    let mut b = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(r.into_reader(), 1_500_000), &mut b)?;
    Ok(Value::String(format!("data:{ty};base64,{}", base64::engine::general_purpose::STANDARD.encode(b))))
}

// ---------- cache ----------

fn fresh(fetched_at: &str, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(fetched_at).map(|t| now - t.with_timezone(&Utc) < Duration::days(MAX_AGE_DAYS)).unwrap_or(false)
}

pub fn cache_get(p: &Project, key: &str) -> Result<Option<(Value, String)>> {
    let row: Option<(String, String)> = p.db.query_row("SELECT json, fetched_at FROM web_cache WHERE key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    Ok(row.and_then(|(j, t)| serde_json::from_str(&j).ok().map(|v| (v, t))))
}
pub fn cache_put(p: &Project, key: &str, v: &Value, now: DateTime<Utc>) -> Result<()> {
    p.db.execute("INSERT INTO web_cache (key, json, fetched_at) VALUES (?1, ?2, ?3)
        ON CONFLICT (key) DO UPDATE SET json = excluded.json, fetched_at = excluded.fetched_at", params![key, v.to_string(), now.to_rfc3339()])?;
    Ok(())
}

/// The cached answer when fresh; else asks the network (when given) and stores it; else whatever is cached, however old.
pub fn cached(p: &Project, key: &str, url: &str, net: Net, now: DateTime<Utc>) -> Option<Value> {
    let have = cache_get(p, key).ok().flatten();
    if let Some((v, t)) = &have { if fresh(t, now) { return Some(v.clone()); } }
    if let Some(f) = net {
        if let Ok(v) = f(url) { let _ = cache_put(p, key, &v, now); return Some(v); }
    }
    have.map(|(v, _)| v)
}

/// True when a key needs fetching (missing or older than 30 days).
fn stale(p: &Project, key: &str, now: DateTime<Utc>) -> bool { !matches!(cache_get(p, key), Ok(Some((_, t))) if fresh(&t, now)) }

fn key_checklist(lat: f64, lon: f64, g: &str) -> String { format!("checklist:{}:{}:{KM}:{g}", fmt1(lat), fmt1(lon)) }
fn key_arrivals(lat: f64, lon: f64) -> String { format!("arrivals:{}:{}", fmt1(lat), fmt1(lon)) }
fn key_areas(lat: f64, lon: f64) -> String { format!("areas:{}:{}:50", fmt1(lat), fmt1(lon)) }
fn key_media(name: &str) -> String { format!("media:{}", norm(name)) }

// ---------- checklists and matching ----------

#[derive(Debug, Clone, Serialize)]
pub struct Known { pub scientific: String, pub vernacular: Option<String>, pub group: String, pub records: i64 }

/// The species known within KM of one point, all groups, with name lookup.
#[derive(Debug, Default)]
pub struct Checklist {
    pub species: Vec<Known>,
    /// Per group: (species listed, list cut short by the site).
    pub groups: HashMap<String, (usize, bool)>,
    by_name: HashMap<String, usize>,
    by_binomial: HashMap<String, usize>,
}

impl Checklist {
    pub fn from_answers(answers: &[(String, Value)]) -> Self {
        let mut c = Checklist::default();
        for (g, v) in answers {
            let list = v["species"].as_array().cloned().unwrap_or_default();
            c.groups.insert(g.clone(), (list.len(), v["truncated"].as_bool().unwrap_or(false)));
            for s in list {
                let Some(name) = s["scientificName"].as_str() else { continue };
                let i = c.species.len();
                c.by_name.entry(norm(name)).or_insert(i);
                c.by_binomial.entry(binomial(name)).or_insert(i);
                c.species.push(Known { scientific: name.into(), vernacular: s["vernacularName"].as_str().map(String::from),
                    group: s["group"].as_str().unwrap_or(g).into(), records: s["records"].as_i64().unwrap_or(0) });
            }
        }
        c
    }
    /// Case-insensitive; a name with a subspecies also matches its species, and the other way round.
    pub fn find(&self, name: &str) -> Option<&Known> {
        self.by_name.get(&norm(name)).or_else(|| self.by_binomial.get(&binomial(name))).map(|&i| &self.species[i])
    }
    pub fn is_empty(&self) -> bool { self.groups.is_empty() }
}

/// The nearby checklist for a point, from cache or network; groups that could not be had are left out.
pub fn checklist(p: &Project, lat: f64, lon: f64, net: Net, now: DateTime<Utc>) -> Checklist {
    let answers: Vec<(String, Value)> = GROUPS.iter()
        .filter_map(|g| cached(p, &key_checklist(lat, lon, g), &checklist_url(lat, lon, g), net, now).map(|v| (g.to_string(), v))).collect();
    Checklist::from_answers(&answers)
}

/// How a species stands at one recorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// In GBIF records within KM.
    Known,
    /// A bird (or likely bird) missing from a long nearby list: check carefully.
    Unusual,
    /// Missing, but its group has few records nearby, so that says little.
    Few,
    /// No nearby list available (never fetched, offline, or the setting is off).
    Unknown,
}

/// The species' class from the media answer's iconic taxon.
fn group_from_iconic(iconic: &str) -> Option<&'static str> {
    match iconic { "Aves" => Some("avian"), "Amphibia" => Some("amphibian"), "Insecta" => Some("insect"), "Mammalia" => Some("bat"), _ => None }
}

/// Status of a species at a recorder. `group` is the species' class when known (from the media answer or the model).
pub fn status<'a>(c: &'a Checklist, name: &str, group: Option<&str>) -> (Status, Option<&'a Known>) {
    if c.is_empty() { return (Status::Unknown, None); }
    if let Some(k) = c.find(name) { return (Status::Known, Some(k)); }
    let g = group.unwrap_or("avian");
    let strong = g == "avian" && matches!(c.groups.get("avian"), Some(&(n, false)) if n >= MIN_LIST_FOR_WARNING);
    (if strong { Status::Unusual } else { Status::Few }, None)
}

fn model_group(model: &str) -> Option<&'static str> { if model.starts_with("batdetect") { Some("bat") } else { None } }

fn cached_group(p: &Project, name: &str) -> Option<&'static str> {
    cache_get(p, &key_media(name)).ok().flatten().and_then(|(v, _)| v["iconic"].as_str().and_then(group_from_iconic))
}

// ---------- media ----------

/// Media answers for names (cache first; the rest asked in batches of 40). Missing names are cached as null.
pub fn media(p: &Project, names: &[String], net: Net, now: DateTime<Utc>) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    let mut ask = Vec::new();
    for n in names {
        match cache_get(p, &key_media(n)).ok().flatten() {
            Some((v, t)) => { if !fresh(&t, now) { ask.push(n.clone()); } out.insert(n.clone(), v); }
            None => ask.push(n.clone()),
        }
    }
    if let Some(f) = net {
        for part in ask.chunks(40) {
            let Ok(v) = f(&media_url(part)) else { break }; // offline: keep the old answers
            for n in part {
                // The site keys its answer by the name as asked; match case-insensitively to be safe.
                let m = v.as_object().and_then(|o| o.iter().find(|(k, _)| norm(k) == norm(n)).map(|(_, x)| x.clone())).unwrap_or(Value::Null);
                let _ = cache_put(p, &key_media(n), &m, now);
                out.insert(n.clone(), m);
            }
        }
    }
    out
}

// ---------- what the app shows ----------

#[derive(Debug, Clone, Serialize)]
pub struct Arrival { pub region: Option<String>, pub week: String, pub low: Option<String>, pub high: Option<String>, pub support: f64 }

#[derive(Debug, Clone, Serialize)]
pub struct SpeciesInfo {
    pub status: Status, pub records: Option<i64>, pub group: Option<String>,
    pub arrival: Option<Arrival>,
    pub title: Option<String>, pub line: Option<String>, pub image: Option<String>, pub credit: Option<String>,
    pub url: String,
}

fn first_sentence(s: &str) -> String {
    let s = s.trim();
    let end = s.char_indices().find(|&(i, c)| c == '.' && i >= 30 && s[i + 1..].starts_with(' ')).map(|(i, _)| i + 1).unwrap_or(s.len());
    let one = &s[..end];
    if one.chars().count() > 220 { format!("{}...", one.chars().take(217).collect::<String>().trim_end()) } else { one.to_string() }
}

/// Facts for the Review box: one species at one recorder.
pub fn species_info(p: &Project, scientific: &str, model: &str, lat: Option<f64>, lon: Option<f64>, net: Net, now: DateTime<Utc>) -> SpeciesInfo {
    let m = media(p, &[scientific.to_string()], net, now).remove(scientific).unwrap_or(Value::Null);
    let mut group = m["iconic"].as_str().and_then(group_from_iconic).or_else(|| model_group(model)).map(String::from);
    let (mut st, mut records, mut arrival) = (Status::Unknown, None, None);
    if let (Some(la), Some(lo)) = (lat, lon) {
        let c = checklist(p, la, lo, net, now);
        let (s, k) = status(&c, scientific, group.as_deref());
        st = s;
        if let Some(k) = k { records = Some(k.records); group = Some(k.group.clone()); }
        if let Some(v) = cached(p, &key_arrivals(la, lo), &arrivals_url(la, lo), net, now) {
            arrival = v["arrivals"].as_array().into_iter().flatten().find(|a| {
                let n = a["scientificName"].as_str().unwrap_or("");
                (norm(n) == norm(scientific) || binomial(n) == binomial(scientific)) && a["arrivalSupport"].as_f64().unwrap_or(0.0) >= MIN_ARRIVAL_SUPPORT
            }).and_then(|a| Some(Arrival {
                region: a["region"].as_str().map(String::from), week: a["arrivalWeek"].as_str()?.to_string(),
                low: a["arrivalLow"].as_str().map(String::from), high: a["arrivalHigh"].as_str().map(String::from), support: a["arrivalSupport"].as_f64().unwrap_or(0.0),
            }));
        }
    }
    let image = m["thumbUrl"].as_str().and_then(|u| {
        let key = format!("img:{u}");
        let fetch = |url: &str| http_image(url);
        let net_img: Net = if net.is_some() { Some(&fetch) } else { None };
        cached(p, &key, u, net_img, now).and_then(|v| v.as_str().map(String::from))
    });
    SpeciesInfo {
        status: st, records, group, arrival,
        title: m["title"].as_str().map(String::from), line: m["extract"].as_str().map(first_sentence).filter(|s| !s.is_empty()),
        image, credit: m["attribution"].as_str().map(|a| match m["license"].as_str() { Some(l) => format!("{a}, {l}"), None => a.to_string() }),
        url: species_url(scientific),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Area { pub name: String, pub kind: Option<String>, pub km: f64 }

#[derive(Debug, Clone, Serialize)]
pub struct Around { pub deployment_id: i64, pub species: Option<usize>, pub more: bool, pub areas: Vec<Area> }

/// "Around this recorder": species known within KM and the three nearest protected areas.
pub fn around(p: &Project, deployment_id: i64, lat: f64, lon: f64, net: Net, now: DateTime<Utc>) -> Around {
    let c = checklist(p, lat, lon, net, now);
    let areas = cached(p, &key_areas(lat, lon), &areas_url(lat, lon), net, now).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let mut areas: Vec<Area> = areas.iter().filter_map(|a| Some(Area { name: a["name"].as_str()?.to_string(), kind: a["type"].as_str().map(String::from), km: a["km"].as_f64().unwrap_or(0.0) })).collect();
    areas.sort_by(|a, b| a.km.total_cmp(&b.km));
    areas.truncate(3);
    let names: HashSet<String> = c.species.iter().map(|k| binomial(&k.scientific)).collect();
    Around { deployment_id, species: (!c.is_empty()).then_some(names.len()), more: c.groups.values().any(|g| g.1), areas }
}

/// Species-list markers, from the cache only: species not known within KM of any recorder they were detected at.
/// Label id to Unusual or Few; species known somewhere, or with no data, are left out.
pub fn flags(p: &Project, now: DateTime<Utc>) -> Result<HashMap<i64, Status>> {
    let deps: HashMap<i64, (f64, f64)> = p.deployments()?.into_iter().filter_map(|d| Some((d.id, (d.lat?, d.lon?)))).collect();
    let mut lists: HashMap<i64, Checklist> = HashMap::new();
    for (&id, &(la, lo)) in &deps { lists.insert(id, checklist(p, la, lo, None, now)); }
    let mut st = p.db.prepare("SELECT d.species, s.scientific, f.deployment_id, MIN(d.model) FROM detections d JOIN files f ON f.id = d.file_id
        JOIN species s ON s.id = d.species GROUP BY d.species, f.deployment_id")?;
    let rows: Vec<(i64, String, i64, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    let mut acc: HashMap<i64, Vec<Status>> = HashMap::new();
    let mut groups: HashMap<String, Option<&'static str>> = HashMap::new();
    for (label, name, dep, model) in rows {
        let Some(c) = lists.get(&dep) else { continue };
        let g = *groups.entry(name.clone()).or_insert_with(|| cached_group(p, &name).or_else(|| model_group(&model)));
        acc.entry(label).or_default().push(status(c, &name, g).0);
    }
    Ok(acc.into_iter().filter_map(|(label, v)| {
        if v.contains(&Status::Known) { None }
        else if v.contains(&Status::Unusual) { Some((label, Status::Unusual)) }
        else if v.contains(&Status::Few) { Some((label, Status::Few)) }
        else { None }
    }).collect())
}

/// Fills the cache for every recorder with a position: nearby lists, arrivals, protected areas, then the media answer
/// for species not on a nearby list (to tell birds from other classes). Requests for one recorder run side by side.
/// Returns how many answers were fetched.
pub fn prefetch(p: &Project, net: &(dyn Fn(&str) -> Result<Value> + Sync), now: DateTime<Utc>, mut on_recorder: impl FnMut()) -> Result<usize> {
    let mut fetched = 0;
    for d in p.deployments()? {
        let (Some(la), Some(lo)) = (d.lat, d.lon) else { continue };
        let mut want: Vec<(String, String)> = GROUPS.iter().map(|g| (key_checklist(la, lo, g), checklist_url(la, lo, g))).collect();
        want.push((key_arrivals(la, lo), arrivals_url(la, lo)));
        want.push((key_areas(la, lo), areas_url(la, lo)));
        want.retain(|(k, _)| stale(p, k, now));
        if want.is_empty() { continue; }
        let got: Vec<(String, Result<Value>)> = std::thread::scope(|s| {
            let hs: Vec<_> = want.iter().map(|(k, u)| s.spawn(move || (k.clone(), net(u)))).collect();
            hs.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let mut any = false;
        for (k, r) in got { if let Ok(v) = r { cache_put(p, &k, &v, now)?; fetched += 1; any = true; } }
        if any { on_recorder(); } else { break; } // nothing came back: offline, stop here
    }
    // Species missing from a nearby list: their media answer says which class they are.
    let mut missing: Vec<String> = Vec::new();
    let mut lists: HashMap<i64, Checklist> = HashMap::new();
    let deps: HashMap<i64, (f64, f64)> = p.deployments()?.into_iter().filter_map(|d| Some((d.id, (d.lat?, d.lon?)))).collect();
    let mut st = p.db.prepare("SELECT DISTINCT s.scientific, f.deployment_id FROM detections d JOIN files f ON f.id = d.file_id JOIN species s ON s.id = d.species")?;
    let rows: Vec<(String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    for (name, dep) in rows {
        let Some(&(la, lo)) = deps.get(&dep) else { continue };
        let c = lists.entry(dep).or_insert_with(|| checklist(p, la, lo, None, now));
        if !c.is_empty() && c.find(&name).is_none() && !missing.contains(&name) && stale(p, &key_media(&name), now) { missing.push(name); }
    }
    if !missing.is_empty() {
        let f = |u: &str| net(u);
        fetched += media(p, &missing, Some(&f), now).len();
    }
    Ok(fetched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(&dir.path().join("t.wlnw")).unwrap();
        (dir, p)
    }

    #[test]
    fn rounding() {
        assert_eq!(fmt1(51.5349), "51.5");
        assert_eq!(fmt1(-0.04), "0.0");
        assert_eq!(fmt1(-0.06), "-0.1");
        assert_eq!(fmt1(12.25), "12.3");
        assert_eq!(fmt1(-33.86), "-33.9");
        assert!(checklist_url(51.5349, -0.127, "avian").ends_with("lat=51.5&lon=-0.1&km=25&group=avian"));
        assert_eq!(media_url(&["Turdus merula".into(), "Pica pica".into()]), format!("{SITE}/api/v1/media?names=Turdus%20merula%2CPica%20pica"));
    }

    #[test]
    fn names() {
        let c = Checklist::from_answers(&[("avian".into(), json!({ "species": [
            { "scientificName": "Turdus merula", "group": "avian", "records": 12 },
            { "scientificName": "Motacilla alba yarrellii", "group": "avian", "records": 3 } ], "truncated": false }))]);
        assert_eq!(c.find("turdus  MERULA").unwrap().records, 12);
        assert_eq!(c.find("Turdus merula merula").unwrap().records, 12, "subspecies falls back to the species");
        assert_eq!(c.find("Motacilla alba").unwrap().records, 3, "species matches a listed subspecies");
        assert!(c.find("Turdus philomelos").is_none());
        assert_eq!(binomial(" Parus  major major "), "parus major");
        assert_eq!(species_url("Erithacus Rubecula"), "https://wlnw.org/species/erithacus-rubecula");
    }

    #[test]
    fn statuses() {
        let birds: Vec<Value> = (0..40).map(|i| json!({ "scientificName": format!("Avis s{i}"), "records": 1 })).collect();
        let c = Checklist::from_answers(&[("avian".into(), json!({ "species": birds, "truncated": false })), ("bat".into(), json!({ "species": [] }))]);
        assert_eq!(status(&c, "Avis s3", None).0, Status::Known);
        assert_eq!(status(&c, "Corvus corax", None).0, Status::Unusual);
        assert_eq!(status(&c, "Pipistrellus pygmaeus", Some("bat")).0, Status::Few);
        assert_eq!(status(&Checklist::default(), "Corvus corax", None).0, Status::Unknown);
        let short = Checklist::from_answers(&[("avian".into(), json!({ "species": [{ "scientificName": "Avis s1" }] }))]);
        assert_eq!(status(&short, "Corvus corax", None).0, Status::Few, "a short list says little");
    }

    #[test]
    fn cache_expiry() {
        let (_d, p) = project();
        let calls = Cell::new(0);
        let f = |_: &str| -> Result<Value> { calls.set(calls.get() + 1); Ok(json!({ "n": calls.get() })) };
        let t0 = Utc::now();
        assert_eq!(cached(&p, "k", "u", Some(&f), t0).unwrap()["n"], 1);
        assert_eq!(cached(&p, "k", "u", Some(&f), t0 + Duration::days(29)).unwrap()["n"], 1, "fresh: no request");
        assert_eq!(calls.get(), 1);
        assert_eq!(cached(&p, "k", "u", Some(&f), t0 + Duration::days(31)).unwrap()["n"], 2, "older than 30 days: asked again");
        let down = |_: &str| -> Result<Value> { bail!("offline") };
        assert_eq!(cached(&p, "k", "u", Some(&down), t0 + Duration::days(90)).unwrap()["n"], 2, "offline: the old answer is kept");
        assert_eq!(cached(&p, "k", "u", None, t0 + Duration::days(90)).unwrap()["n"], 2);
        assert!(cached(&p, "other", "u", None, t0).is_none());
    }

    #[test]
    fn arrivals_need_support() {
        let (_d, p) = project();
        let now = Utc::now();
        cache_put(&p, &key_arrivals(51.5, -0.1), &json!({ "arrivals": [
            { "scientificName": "Cygnus columbianus", "arrivalWeek": "2026-10-05", "arrivalSupport": 0.4 },
            { "scientificName": "Hirundo rustica", "arrivalWeek": "2026-04-06", "arrivalLow": "2026-03-30", "arrivalHigh": "2026-04-13", "arrivalSupport": 0.9 } ] }), now).unwrap();
        assert!(species_info(&p, "Cygnus columbianus", "birdnet-2.4", Some(51.5), Some(-0.1), None, now).arrival.is_none());
        let a = species_info(&p, "Hirundo rustica rustica", "birdnet-2.4", Some(51.52), Some(-0.12), None, now).arrival.unwrap();
        assert_eq!(a.week, "2026-04-06");
        assert_eq!(first_sentence("The European robin is a small passerine. It sings."), "The European robin is a small passerine.");
        assert_eq!(first_sentence("St. Kilda wren"), "St. Kilda wren");
    }
}
