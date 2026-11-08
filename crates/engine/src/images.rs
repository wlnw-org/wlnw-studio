//! Camera-trap images in a project: found alongside recordings when a folder is added, run through SpeciesNet, reviewed
//! box by box, exported as Camtrap DP and inside the Darwin Core Archive and WDX exports. Images with people in them are
//! never exported or published.
use crate::camtrap::{self, Found};
use crate::models::Classifier;
use crate::photo::PhotoMeta;
use crate::review::{fit, store_fit, BANDS, TARGET};
use crate::store::Project;
use anyhow::{bail, Context, Result};
use chrono::{Duration, NaiveDateTime};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS images (
  id INTEGER PRIMARY KEY, deployment_id INTEGER NOT NULL REFERENCES deployments(id), path TEXT NOT NULL UNIQUE,
  taken_local TEXT, time_source TEXT, bytes INTEGER, width INTEGER, height INTEGER, lat REAL, lon REAL);
-- One row per image and model: 'done' or 'error'.
CREATE TABLE IF NOT EXISTS image_models (image_id INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE, model TEXT NOT NULL,
  status TEXT NOT NULL, error TEXT, processed_at TEXT NOT NULL, PRIMARY KEY (image_id, model));
-- One row per detector box SpeciesNet classified: box as fractions of the (EXIF-turned) image, kind = animal, human,
-- vehicle, blank or unknown; species for animals; label, score and source as SpeciesNet's ensemble gave them; top = its
-- five best classes.
CREATE TABLE IF NOT EXISTS image_boxes (
  id INTEGER PRIMARY KEY, image_id INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE, model TEXT NOT NULL,
  category TEXT NOT NULL, conf REAL NOT NULL, x REAL NOT NULL, y REAL NOT NULL, w REAL NOT NULL, h REAL NOT NULL,
  kind TEXT NOT NULL, species INTEGER REFERENCES species(id), label TEXT, score REAL NOT NULL, source TEXT, top TEXT);
CREATE INDEX IF NOT EXISTS image_boxes_image ON image_boxes (image_id);
CREATE INDEX IF NOT EXISTS image_boxes_species ON image_boxes (species, score);
CREATE TABLE IF NOT EXISTS image_reviews (box_id INTEGER PRIMARY KEY REFERENCES image_boxes(id) ON DELETE CASCADE,
  verdict TEXT NOT NULL CHECK (verdict IN ('yes','no','unsure')), at TEXT NOT NULL);
"#;

/// Adds one image found by a scan to deployment `dep` (scan.rs decides the deployment, as for recordings).
pub fn add(tx: &Transaction, dep: i64, path: &str, m: &PhotoMeta, bytes: i64) -> Result<()> {
    tx.execute("INSERT INTO images (deployment_id, path, taken_local, time_source, bytes, width, height, lat, lon) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![dep, path, m.taken_local.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()), m.taken_local.map(|_| "exif"), bytes,
            m.width.map(|v| v as i64), m.height.map(|v| v as i64), m.lat, m.lon])?;
    Ok(())
}

/// Installed models that read images.
pub fn image_models(models: &[Arc<dyn Classifier>]) -> Vec<Arc<dyn Classifier>> { models.iter().filter(|m| m.images().is_some()).cloned().collect() }

#[derive(Debug, Clone, Serialize, Default)]
pub struct Counts {
    pub total: i64, pub done: i64, pub failed: i64,
    /// Processed images by what SpeciesNet saw (an image can count in more than one).
    pub animals: i64, pub people: i64, pub vehicles: i64, pub empty: i64,
    pub boxes: i64,
}

const KINDS: &str = "('animal','human','vehicle')";

pub fn counts(p: &Project, model: &str) -> Result<Counts> {
    let one = |sql: &str| -> Result<i64> { Ok(p.db.query_row(sql, [model], |r| r.get(0))?) };
    Ok(Counts {
        total: p.db.query_row("SELECT COUNT(*) FROM images", [], |r| r.get(0))?,
        done: one("SELECT COUNT(*) FROM image_models WHERE model = ?1 AND status = 'done'")?,
        failed: one("SELECT COUNT(*) FROM image_models WHERE model = ?1 AND status = 'error'")?,
        animals: one("SELECT COUNT(DISTINCT image_id) FROM image_boxes WHERE model = ?1 AND kind = 'animal'")?,
        people: one("SELECT COUNT(DISTINCT image_id) FROM image_boxes WHERE model = ?1 AND kind = 'human'")?,
        vehicles: one("SELECT COUNT(DISTINCT image_id) FROM image_boxes WHERE model = ?1 AND kind = 'vehicle'")?,
        empty: one(&format!("SELECT COUNT(*) FROM image_models m WHERE m.model = ?1 AND m.status = 'done'
            AND NOT EXISTS (SELECT 1 FROM image_boxes b WHERE b.image_id = m.image_id AND b.model = m.model AND b.kind IN {KINDS})"))?,
        boxes: one("SELECT COUNT(*) FROM image_boxes WHERE model = ?1")?,
    })
}

/// Images still to run through `model`.
pub fn todo(p: &Project, model: &str) -> Result<i64> {
    Ok(p.db.query_row("SELECT COUNT(*) FROM images i WHERE NOT EXISTS (SELECT 1 FROM image_models m WHERE m.image_id = i.id AND m.model = ?1)", [model], |r| r.get(0))?)
}

/// Country (ISO 3166-1 alpha-3) and US state of a deployment, for SpeciesNet's geofencing.
fn place(p: &Project, dep: i64) -> (Option<String>, Option<String>) {
    p.db.query_row("SELECT country, admin1 FROM deployments WHERE id = ?1", [dep], |r| Ok((r.get(0)?, r.get(1)?))).unwrap_or((None, None))
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct RunStats { pub images: i64, pub failed: i64, pub boxes: i64, pub seconds: f64 }

/// Runs every image not yet processed through each image model. `on_image(done, total, path)` before each image.
/// Each image is committed on its own, so stopping loses nothing.
pub fn run(p: &Project, models: &[Arc<dyn Classifier>], stop: &AtomicBool, mut on_image: impl FnMut(i64, i64, &str)) -> Result<RunStats> {
    let mut stats = RunStats::default();
    let started = std::time::Instant::now();
    for m in image_models(models) {
        let net = m.images().unwrap();
        let id = m.info().id.clone();
        let rows: Vec<(i64, String, i64)> = {
            let mut st = p.db.prepare("SELECT i.id, i.path, i.deployment_id FROM images i WHERE NOT EXISTS
                (SELECT 1 FROM image_models m WHERE m.image_id = i.id AND m.model = ?1) ORDER BY i.deployment_id, i.taken_local, i.path")?;
            let rows = st.query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            rows
        };
        let total = rows.len() as i64;
        let mut places: HashMap<i64, (Option<String>, Option<String>)> = HashMap::new();
        let mut species: HashMap<String, i64> = HashMap::new();
        // Reading and resizing the next image overlaps with the networks on this one.
        let (tx_img, rx_img) = std::sync::mpsc::sync_channel::<(usize, Result<camtrap::Rgb>)>(2);
        let paths: Vec<String> = rows.iter().map(|r| r.1.clone()).collect();
        let stop_reader = Arc::new(AtomicBool::new(false));
        let sr = stop_reader.clone();
        let reader = std::thread::spawn(move || {
            for (k, path) in paths.iter().enumerate() {
                if sr.load(Ordering::Relaxed) { break; }
                if tx_img.send((k, camtrap::load(Path::new(path)))).is_err() { break; }
            }
        });
        for (k, (image_id, path, dep)) in rows.iter().enumerate() {
            if stop.load(Ordering::Relaxed) { break; }
            on_image(k as i64, total, path);
            let (got, img) = rx_img.recv().context("image reader stopped")?;
            debug_assert_eq!(got, k);
            let (country, admin1) = places.entry(*dep).or_insert_with(|| place(p, *dep)).clone();
            let res = img.and_then(|im| Ok((im.w, im.h, net.process(&im, country.as_deref(), admin1.as_deref())?)));
            match res {
                Ok((w, h, found)) => {
                    let tx = p.db.unchecked_transaction()?;
                    tx.execute("DELETE FROM image_boxes WHERE image_id = ?1 AND model = ?2", params![image_id, id])?;
                    let n = store_boxes(p, &tx, *image_id, &id, &found, &mut species)?;
                    tx.execute("UPDATE images SET width = ?2, height = ?3 WHERE id = ?1", params![image_id, w as i64, h as i64])?;
                    tx.execute("INSERT OR REPLACE INTO image_models (image_id, model, status, error, processed_at) VALUES (?1, ?2, 'done', NULL, datetime('now'))", params![image_id, id])?;
                    tx.commit()?;
                    stats.images += 1;
                    stats.boxes += n;
                }
                Err(e) => {
                    p.db.execute("INSERT OR REPLACE INTO image_models (image_id, model, status, error, processed_at) VALUES (?1, ?2, 'error', ?3, datetime('now'))",
                        params![image_id, id, format!("{e:#}")])?;
                    stats.failed += 1;
                }
            }
        }
        stop_reader.store(true, Ordering::Relaxed);
        drop(rx_img);
        let _ = reader.join();
        on_image(total, total, "");
    }
    stats.seconds = started.elapsed().as_secs_f64();
    Ok(stats)
}

/// Writes the classified boxes of one image (unclassified ones, below SpeciesNet's 0.1, are not kept). Returns how many.
fn store_boxes(p: &Project, tx: &Transaction, image_id: i64, model: &str, found: &[Found], species: &mut HashMap<String, i64>) -> Result<i64> {
    let mut ins = tx.prepare("INSERT INTO image_boxes (image_id, model, category, conf, x, y, w, h, kind, species, label, score, source, top)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)")?;
    let mut n = 0;
    for f in found {
        let Some(pr) = &f.processed else { continue };
        let t = camtrap::taxon(&pr.label);
        let sp = if t.kind == "animal" {
            Some(match species.get(&t.scientific) { Some(&s) => s, None => { let s = p.species_id(&t.scientific, &t.common)?; species.insert(t.scientific.clone(), s); s } })
        } else { None };
        let top: Vec<(String, f64)> = f.top.iter().map(|(l, s)| { let t = camtrap::taxon(l); (if t.common.is_empty() { t.scientific } else { t.common }, (*s * 10000.0).round() / 10000.0) }).collect();
        let b = f.det.bbox;
        ins.execute(params![image_id, model, camtrap::CLASSES[f.det.class], f.det.conf as f64, b[0], b[1], b[2], b[3], t.kind, sp, pr.label, pr.score, pr.source, serde_json::to_string(&top)?])?;
        n += 1;
    }
    Ok(n)
}

// ---------------------------------------------------------------------------------------------------------------------
// Review

#[derive(Debug, Clone, Serialize)]
pub struct SpeciesRow {
    pub label: i64, pub model: String, pub scientific: String, pub common: String, pub boxes: i64, pub images: i64, pub kept: i64,
    pub reviewed: i64, pub yes: i64, pub no: i64, pub threshold: f64, pub threshold_source: String, pub precision: Option<f64>, pub max_score: f64,
}

/// Every animal taxon SpeciesNet named, per model, most boxes first.
pub fn species(p: &Project) -> Result<Vec<SpeciesRow>> {
    let mut st = p.db.prepare(
        "SELECT b.species, b.model, s.scientific, s.common, COUNT(*), COUNT(DISTINCT b.image_id), COUNT(r.verdict), COUNT(*) FILTER (WHERE r.verdict = 'yes'),
                COUNT(*) FILTER (WHERE r.verdict = 'no'), t.threshold, t.source, t.precision, MAX(b.score)
         FROM image_boxes b JOIN species s ON s.id = b.species LEFT JOIN image_reviews r ON r.box_id = b.id
         LEFT JOIN thresholds t ON t.species = b.species AND t.model = b.model
         WHERE b.kind = 'animal' GROUP BY b.species, b.model ORDER BY COUNT(*) DESC")?;
    let rows: Vec<SpeciesRow> = st.query_map([], |r| Ok(SpeciesRow {
        label: r.get(0)?, model: r.get(1)?, scientific: r.get(2)?, common: r.get(3)?, boxes: r.get(4)?, images: r.get(5)?, kept: 0, reviewed: r.get(6)?,
        yes: r.get(7)?, no: r.get(8)?, threshold: r.get::<_, Option<f64>>(9)?.unwrap_or(f64::NAN),
        threshold_source: r.get::<_, Option<String>>(10)?.unwrap_or_else(|| "default".into()), precision: r.get(11)?, max_score: r.get(12)?,
    }))?.collect::<Result<_, _>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for mut row in rows {
        if row.threshold.is_nan() { row.threshold = p.default_threshold(&row.model); }
        row.kept = p.db.query_row("SELECT COUNT(*) FROM image_boxes b LEFT JOIN image_reviews r ON r.box_id = b.id
            WHERE b.species = ?1 AND b.model = ?2 AND b.kind = 'animal' AND ((b.score >= ?3 AND COALESCE(r.verdict, '') <> 'no') OR r.verdict = 'yes')",
            params![row.label, row.model, row.threshold], |r| r.get(0))?;
        out.push(row);
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct BoxRow {
    pub id: i64, pub model: String, pub kind: String, pub category: String, pub conf: f64, pub bbox: [f64; 4], pub species: Option<i64>,
    pub scientific: String, pub common: String, pub score: f64, pub source: String, pub verdict: Option<String>, pub top: Vec<(String, f64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImageItem {
    pub id: i64, pub path: String, pub deployment_id: i64, pub site: String, pub taken_local: Option<String>,
    /// When the photo was taken, in UTC, when the camera's time and its deployment's time zone are known.
    pub utc: Option<NaiveDateTime>, pub width: Option<i64>, pub height: Option<i64>, pub lat: Option<f64>, pub lon: Option<f64>,
    pub boxes: Vec<BoxRow>,
    /// The box this tile asks about (species review), if any.
    pub focus: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Query {
    /// Species review: boxes of this species from this model, unreviewed first, spread over the score bands.
    #[serde(default)] pub species: Option<i64>,
    #[serde(default)] pub model: Option<String>,
    /// Or images by what was seen: animals, people, vehicles, empty, failed, all.
    #[serde(default)] pub show: Option<String>,
    #[serde(default)] pub offset: usize,
    #[serde(default)] pub limit: usize,
}

fn boxes_of(p: &Project, image: i64) -> Result<Vec<BoxRow>> {
    let mut st = p.db.prepare("SELECT b.id, b.model, b.kind, b.category, b.conf, b.x, b.y, b.w, b.h, b.species, COALESCE(s.scientific, ''), COALESCE(s.common, ''),
        b.score, COALESCE(b.source, ''), r.verdict, COALESCE(b.top, '[]') FROM image_boxes b LEFT JOIN species s ON s.id = b.species
        LEFT JOIN image_reviews r ON r.box_id = b.id WHERE b.image_id = ?1 ORDER BY b.conf DESC")?;
    let rows = st.query_map([image], |r| {
        let top: String = r.get(15)?;
        Ok(BoxRow { id: r.get(0)?, model: r.get(1)?, kind: r.get(2)?, category: r.get(3)?, conf: r.get(4)?, bbox: [r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?],
            species: r.get(9)?, scientific: r.get(10)?, common: r.get(11)?, score: r.get(12)?, source: r.get(13)?, verdict: r.get(14)?,
            top: serde_json::from_str(&top).unwrap_or_default() })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn item(p: &Project, image: i64, focus: Option<i64>) -> Result<ImageItem> {
    let (id, path, dep, site, taken, tz, w, h, lat, lon): (i64, String, i64, String, Option<String>, i32, Option<i64>, Option<i64>, Option<f64>, Option<f64>) =
        p.db.query_row("SELECT i.id, i.path, i.deployment_id, d.name, i.taken_local, d.tz_offset_min, i.width, i.height, COALESCE(i.lat, d.lat), COALESCE(i.lon, d.lon)
            FROM images i JOIN deployments d ON d.id = i.deployment_id WHERE i.id = ?1", [image],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?)))?;
    let utc = taken.as_deref().and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()).map(|t| t - Duration::minutes(tz as i64));
    Ok(ImageItem { id, path, deployment_id: dep, site, taken_local: taken, utc, width: w, height: h, lat, lon, boxes: boxes_of(p, image)?, focus })
}

/// Tiles to review: for a species, its boxes (unreviewed first, a few from every score band, highest first, then the
/// reviewed ones); otherwise images by what was seen, in time order.
pub fn queue(p: &Project, q: &Query) -> Result<Vec<ImageItem>> {
    let limit = if q.limit == 0 { 60 } else { q.limit.min(500) };
    if let Some(sp) = q.species {
        let model = q.model.clone().unwrap_or_else(|| camtrap::ID.to_string());
        let mut picked: Vec<(i64, i64)> = Vec::new();
        let per = limit.div_ceil(9).max(1) as i64;
        for b in (0..9).rev() {
            let mut st = p.db.prepare("SELECT b.id, b.image_id FROM image_boxes b LEFT JOIN image_reviews r ON r.box_id = b.id
                WHERE b.species = ?1 AND b.model = ?2 AND b.kind = 'animal' AND r.verdict IS NULL AND b.score >= ?3 AND b.score < ?4
                ORDER BY (b.id * 2654435761) % 1000003 LIMIT ?5")?;
            let rows: Vec<(i64, i64)> = st.query_map(params![sp, model, BANDS[b], BANDS[b + 1], per], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            picked.extend(rows);
        }
        // Fill up with more unreviewed boxes, then reviewed ones, so a species with few boxes shows them all.
        let mut st = p.db.prepare("SELECT b.id, b.image_id FROM image_boxes b LEFT JOIN image_reviews r ON r.box_id = b.id
            WHERE b.species = ?1 AND b.model = ?2 AND b.kind = 'animal' ORDER BY r.verdict IS NOT NULL, b.score DESC LIMIT ?3")?;
        let more: Vec<(i64, i64)> = st.query_map(params![sp, model, (limit * 2) as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        for m in more { if picked.len() >= limit { break; } if !picked.contains(&m) { picked.push(m); } }
        picked.truncate(limit);
        return picked.into_iter().skip(q.offset.min(0)).map(|(b, i)| item(p, i, Some(b))).collect();
    }
    let cond = match q.show.as_deref().unwrap_or("animals") {
        "animals" => "EXISTS (SELECT 1 FROM image_boxes b WHERE b.image_id = i.id AND b.kind = 'animal')",
        "people" => "EXISTS (SELECT 1 FROM image_boxes b WHERE b.image_id = i.id AND b.kind = 'human')",
        "vehicles" => "EXISTS (SELECT 1 FROM image_boxes b WHERE b.image_id = i.id AND b.kind = 'vehicle')",
        "empty" => "EXISTS (SELECT 1 FROM image_models m WHERE m.image_id = i.id AND m.status = 'done') AND NOT EXISTS (SELECT 1 FROM image_boxes b WHERE b.image_id = i.id AND b.kind IN ('animal','human','vehicle'))",
        "failed" => "EXISTS (SELECT 1 FROM image_models m WHERE m.image_id = i.id AND m.status = 'error')",
        "todo" => "NOT EXISTS (SELECT 1 FROM image_models m WHERE m.image_id = i.id)",
        "all" => "1",
        x => bail!("unknown image filter {x}"),
    };
    let focus_kind = match q.show.as_deref() { Some("people") => Some("human"), Some("vehicles") => Some("vehicle"), Some("animals") | None => Some("animal"), _ => None };
    let mut st = p.db.prepare(&format!("SELECT i.id FROM images i WHERE {cond} ORDER BY i.taken_local, i.path LIMIT ?1 OFFSET ?2"))?;
    let ids: Vec<i64> = st.query_map(params![limit as i64, q.offset as i64], |r| r.get(0))?.collect::<Result<_, _>>()?;
    ids.into_iter().map(|i| {
        let mut it = item(p, i, None)?;
        it.focus = focus_kind.and_then(|k| it.boxes.iter().find(|b| b.kind == k).map(|b| b.id));
        Ok(it)
    }).collect()
}

/// How many images each filter holds, for the filter buttons.
pub fn filter_counts(p: &Project) -> Result<BTreeMap<&'static str, i64>> {
    let mut out = BTreeMap::new();
    for show in ["animals", "people", "vehicles", "empty", "failed", "todo", "all"] {
        let q = match show {
            "animals" => "SELECT COUNT(DISTINCT image_id) FROM image_boxes WHERE kind = 'animal'",
            "people" => "SELECT COUNT(DISTINCT image_id) FROM image_boxes WHERE kind = 'human'",
            "vehicles" => "SELECT COUNT(DISTINCT image_id) FROM image_boxes WHERE kind = 'vehicle'",
            "empty" => "SELECT COUNT(*) FROM images i WHERE EXISTS (SELECT 1 FROM image_models m WHERE m.image_id = i.id AND m.status = 'done') AND NOT EXISTS (SELECT 1 FROM image_boxes b WHERE b.image_id = i.id AND b.kind IN ('animal','human','vehicle'))",
            "failed" => "SELECT COUNT(DISTINCT image_id) FROM image_models WHERE status = 'error'",
            "todo" => "SELECT COUNT(*) FROM images i WHERE NOT EXISTS (SELECT 1 FROM image_models m WHERE m.image_id = i.id)",
            _ => "SELECT COUNT(*) FROM images",
        };
        out.insert(show, p.db.query_row(q, [], |r| r.get(0))?);
    }
    Ok(out)
}

/// A verdict on one box; refits that species' threshold for its model (animals only).
pub fn set_verdict(p: &Project, box_id: i64, verdict: Option<&str>) -> Result<()> {
    match verdict {
        None => { p.db.execute("DELETE FROM image_reviews WHERE box_id = ?1", [box_id])?; }
        Some(v @ ("yes" | "no" | "unsure")) => {
            p.db.execute("INSERT INTO image_reviews (box_id, verdict, at) VALUES (?1, ?2, datetime('now'))
                ON CONFLICT (box_id) DO UPDATE SET verdict = excluded.verdict, at = excluded.at", params![box_id, v])?;
        }
        Some(_) => bail!("verdict must be yes, no or unsure"),
    }
    let (species, model): (Option<i64>, String) = p.db.query_row("SELECT species, model FROM image_boxes WHERE id = ?1", [box_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    if let Some(s) = species { fit_threshold(p, s, &model)?; }
    Ok(())
}

pub fn fit_threshold(p: &Project, species: i64, model: &str) -> Result<Option<f64>> {
    let mut st = p.db.prepare("SELECT b.score, r.verdict FROM image_boxes b LEFT JOIN image_reviews r ON r.box_id = b.id WHERE b.species = ?1 AND b.model = ?2 AND b.kind = 'animal'")?;
    let rows: Vec<(f64, Option<String>)> = st.query_map(params![species, model], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    store_fit(p, species, model, fit(&rows, TARGET))
}

/// A JPEG of the image (EXIF orientation applied, so boxes line up), at most `max` pixels on its longer side. For the
/// screen only, so it uses the faster decoder.
pub fn thumbnail(path: &Path, max: u32) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    let img = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format()?.decode()?.to_rgb8();
    let (w, h) = (img.width(), img.height());
    let scale = (max as f64 / w.max(h) as f64).min(1.0);
    let img = if scale < 1.0 { image::imageops::thumbnail(&img, ((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1)) } else { img };
    let small = camtrap::orient(camtrap::Rgb { w: img.width() as usize, h: img.height() as usize, px: img.into_raw() }, crate::photo::orientation(&bytes));
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode(&small.px, small.w as u32, small.h as u32, image::ExtendedColorType::Rgb8)?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------------------
// What is exported

/// One accepted animal box, with what exports need.
#[derive(Debug, Clone, Serialize)]
pub struct Obs {
    pub box_id: i64, pub image_id: i64, pub path: String, pub deployment_id: i64, pub site: String,
    pub taken_local: Option<NaiveDateTime>, pub tz_offset_min: i32, pub utc: Option<NaiveDateTime>, pub lat: Option<f64>, pub lon: Option<f64>,
    pub species: i64, pub scientific: String, pub common: String, pub rank: String, pub model: String, pub score: f64, pub conf: f64,
    pub bbox: [f64; 4], pub verdict: Option<String>, pub width: Option<i64>, pub height: Option<i64>,
}

/// Images with a person in them (a human box nobody rejected): never exported or published.
pub fn with_people(p: &Project) -> Result<std::collections::HashSet<i64>> {
    let mut st = p.db.prepare("SELECT DISTINCT b.image_id FROM image_boxes b LEFT JOIN image_reviews r ON r.box_id = b.id WHERE b.kind = 'human' AND COALESCE(r.verdict, '') <> 'no'")?;
    let rows = st.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Animal boxes to export or publish: at or above their species' threshold for their model and not rejected, or
/// reviewed yes; never from an image with people in it.
pub fn accepted(p: &Project) -> Result<Vec<Obs>> {
    let people = with_people(p)?;
    let mut st = p.db.prepare("SELECT b.id, b.image_id, i.path, i.deployment_id, d.name, i.taken_local, d.tz_offset_min, COALESCE(i.lat, d.lat), COALESCE(i.lon, d.lon),
            b.species, s.scientific, s.common, COALESCE(b.label, ''), b.model, b.score, b.conf, b.x, b.y, b.w, b.h, r.verdict, i.width, i.height
        FROM image_boxes b JOIN images i ON i.id = b.image_id JOIN deployments d ON d.id = i.deployment_id JOIN species s ON s.id = b.species
        LEFT JOIN image_reviews r ON r.box_id = b.id WHERE b.kind = 'animal' AND COALESCE(r.verdict, '') <> 'no' ORDER BY i.taken_local, i.path, b.conf DESC")?;
    let mut cache: HashMap<(i64, String), f64> = HashMap::new();
    let rows: Vec<Obs> = st.query_map([], |r| {
        let taken: Option<String> = r.get(5)?;
        let taken = taken.as_deref().and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok());
        let tz: i32 = r.get(6)?;
        let label: String = r.get(12)?;
        Ok(Obs {
            box_id: r.get(0)?, image_id: r.get(1)?, path: r.get(2)?, deployment_id: r.get(3)?, site: r.get(4)?, taken_local: taken, tz_offset_min: tz,
            utc: taken.map(|t| t - Duration::minutes(tz as i64)), lat: r.get(7)?, lon: r.get(8)?, species: r.get(9)?, scientific: r.get(10)?, common: r.get(11)?,
            rank: camtrap::taxon(&label).rank.to_string(), model: r.get(13)?, score: r.get(14)?, conf: r.get(15)?,
            bbox: [r.get(16)?, r.get(17)?, r.get(18)?, r.get(19)?], verdict: r.get(20)?, width: r.get(21)?, height: r.get(22)?,
        })
    })?.collect::<Result<_, _>>()?;
    Ok(rows.into_iter().filter(|o| !people.contains(&o.image_id))
        .filter(|o| o.verdict.as_deref() == Some("yes") || o.score >= *cache.entry((o.species, o.model.clone())).or_insert_with(|| p.threshold(o.species, &o.model)))
        .collect())
}

/// Accepted boxes grouped per image and taxon: (first box, how many), for record-per-sighting exports.
pub fn per_image(obs: &[Obs]) -> Vec<(&Obs, usize)> {
    let mut out: Vec<(&Obs, usize)> = Vec::new();
    let mut at: HashMap<(i64, i64), usize> = HashMap::new();
    for o in obs {
        match at.get(&(o.image_id, o.species)) {
            Some(&k) => out[k].1 += 1,
            None => { at.insert((o.image_id, o.species), out.len()); out.push((o, 1)); }
        }
    }
    out
}

pub fn model_label(id: &str) -> (&'static str, &'static str) { if id == camtrap::ID { ("SpeciesNet", "4.0.3a") } else { ("SpeciesNet", "4") } }

/// "+05:30" for a UTC offset in minutes.
fn offset_str(min: i32) -> String { format!("{}{:02}:{:02}", if min < 0 { '-' } else { '+' }, min.abs() / 60, min.abs() % 60) }

fn csv_field(v: &str) -> String { if v.contains([',', '"', '\n', '\r']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v.to_string() } }

#[derive(Debug, Default, Serialize)]
pub struct CamtrapReport { pub deployments: usize, pub media: usize, pub observations: usize, pub left_out_people: usize, pub left_out_no_time_or_place: usize }

/// Camtrap DP 1.0 (https://camtrap-dp.tdwg.org): deployments.csv, media.csv, observations.csv and datapackage.json in
/// `dir`. Media and observations are per image and per box (observationLevel media). Images with people are left out;
/// so are images without a time or whose deployment has no place, since Camtrap DP requires both.
pub fn camtrapdp(p: &Project, dir: &Path) -> Result<CamtrapReport> {
    std::fs::create_dir_all(dir)?;
    let info = crate::export::info(p);
    let people = with_people(p)?;
    let mut rep = CamtrapReport::default();
    let accepted = accepted(p)?;
    let mut acc_by_image: HashMap<i64, Vec<&Obs>> = HashMap::new();
    for o in &accepted { acc_by_image.entry(o.image_id).or_default().push(o); }
    let deps = p.deployments()?;
    let processed: std::collections::HashSet<i64> = {
        let mut st = p.db.prepare("SELECT image_id FROM image_models WHERE status = 'done'")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    // Every image: (id, deployment, path, taken local, kinds seen)
    let mut st = p.db.prepare("SELECT i.id, i.deployment_id, i.path, i.taken_local FROM images i ORDER BY i.deployment_id, i.taken_local, i.path")?;
    let images: Vec<(i64, i64, String, Option<String>)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    let mut st = p.db.prepare("SELECT b.image_id, b.kind, b.conf, b.x, b.y, b.w, b.h, b.score, b.model, b.id FROM image_boxes b LEFT JOIN image_reviews r ON r.box_id = b.id
        WHERE b.kind IN ('vehicle', 'unknown') AND COALESCE(r.verdict, '') <> 'no'")?;
    let mut other: HashMap<i64, Vec<(String, [f64; 4], f64, String, i64)>> = HashMap::new();
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, [r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?], r.get::<_, f64>(7)?, r.get::<_, String>(8)?, r.get::<_, i64>(9)?)))? {
        let (img, kind, b, score, model, id) = r?;
        other.entry(img).or_default().push((kind, b, score, model, id));
    }
    let ds = p.setting("dataset_id")?.unwrap_or_else(|| "wlnw-studio".into());
    let mut media = String::from("mediaID,deploymentID,captureMethod,timestamp,filePath,filePublic,fileName,fileMediatype,exifData,favorite,mediaComments\n");
    let mut obs = String::from("observationID,deploymentID,mediaID,eventID,eventStart,eventEnd,observationLevel,observationType,cameraSetupType,scientificName,count,lifeStage,sex,behavior,individualID,individualPositionRadius,individualPositionAngle,individualSpeed,bboxX,bboxY,bboxWidth,bboxHeight,classificationMethod,classifiedBy,classificationTimestamp,classificationProbability,observationTags,observationComments\n");
    let mut span: HashMap<i64, (String, String)> = HashMap::new();
    let mut taxa: BTreeMap<String, String> = BTreeMap::new();
    let (mut t_min, mut t_max): (Option<String>, Option<String>) = (None, None);
    let dep_of: HashMap<i64, &crate::store::Deployment> = deps.iter().map(|d| (d.id, d)).collect();
    for (id, dep, path, taken) in &images {
        if people.contains(id) { rep.left_out_people += 1; continue; }
        let d = dep_of[dep];
        let (Some(taken), Some(_), Some(_)) = (taken.as_deref().and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()), d.lat, d.lon) else { rep.left_out_no_time_or_place += 1; continue };
        let ts = format!("{}{}", taken.format("%Y-%m-%dT%H:%M:%S"), offset_str(d.tz_offset_min));
        let e = span.entry(*dep).or_insert_with(|| (ts.clone(), ts.clone()));
        if ts < e.0 { e.0 = ts.clone(); }
        if ts > e.1 { e.1 = ts.clone(); }
        if t_min.as_ref().is_none_or(|m| &ts < m) { t_min = Some(ts.clone()); }
        if t_max.as_ref().is_none_or(|m| &ts > m) { t_max = Some(ts.clone()); }
        let file = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let mime = if file.to_ascii_lowercase().ends_with(".png") { "image/png" } else { "image/jpeg" };
        let mid = format!("{ds}-m{id}");
        media.push_str(&[mid.clone(), format!("{ds}-d{dep}"), "activityDetection".into(), ts.clone(), csv_field(&file), "false".into(), csv_field(&file), mime.into(), String::new(), String::new(), String::new()].join(","));
        media.push('\n');
        rep.media += 1;
        let mut row = |oid: String, kind: &str, sci: &str, count: &str, b: Option<[f64; 4]>, method: &str, by: &str, prob: Option<f64>| {
            let bb = b.map(|b| b.iter().map(|v| format!("{:.4}", v)).collect::<Vec<_>>()).unwrap_or_else(|| vec![String::new(); 4]);
            obs.push_str(&[oid, format!("{ds}-d{dep}"), mid.clone(), String::new(), ts.clone(), ts.clone(), "media".into(), kind.into(), String::new(), csv_field(sci), count.into(),
                String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), bb[0].clone(), bb[1].clone(), bb[2].clone(), bb[3].clone(),
                method.into(), csv_field(by), String::new(), prob.map(|v| format!("{v:.4}")).unwrap_or_default(), String::new(), String::new()].join(","));
            obs.push('\n');
        };
        let mut n = 0;
        for o in acc_by_image.get(id).into_iter().flatten() {
            let (name, ver) = model_label(&o.model);
            let human = o.verdict.as_deref() == Some("yes");
            row(format!("{ds}-o{}", o.box_id), "animal", &o.scientific, "1", Some(o.bbox), if human { "human" } else { "machine" },
                &if human { info.people.first().cloned().unwrap_or_else(|| "reviewer".into()) } else { format!("{name} {ver}") }, if human { None } else { Some(o.score) });
            taxa.insert(o.scientific.clone(), o.rank.clone());
            n += 1;
        }
        for (kind, b, score, model, bid) in other.get(id).into_iter().flatten() {
            let (name, ver) = model_label(model);
            row(format!("{ds}-o{bid}"), kind, "", "", Some(*b), "machine", &format!("{name} {ver}"), Some(*score));
            n += 1;
        }
        if n == 0 {
            // Processed with nothing kept: blank when SpeciesNet saw nothing, else not classified (below threshold or rejected).
            let blank = processed.contains(id) && !p.db.query_row("SELECT EXISTS (SELECT 1 FROM image_boxes WHERE image_id = ?1 AND kind = 'animal')", [id], |r| r.get::<_, bool>(0))?;
            row(format!("{ds}-m{id}-o"), if blank { "blank" } else { "unclassified" }, "", "", None, if blank { "machine" } else { "" }, if blank { "SpeciesNet 4.0.3a" } else { "" }, None);
        }
        rep.observations += n.max(1);
    }
    let mut depl = String::from("deploymentID,locationID,locationName,latitude,longitude,coordinateUncertainty,deploymentStart,deploymentEnd,setupBy,cameraID,cameraModel,cameraDelay,cameraHeight,cameraDepth,cameraTilt,cameraHeading,detectionDistance,timestampIssues,baitUse,featureType,habitat,deploymentGroups,deploymentTags,deploymentComments\n");
    for d in &deps {
        let Some((start, end)) = span.get(&d.id) else { continue };
        depl.push_str(&[format!("{ds}-d{}", d.id), format!("{ds}-d{}", d.id), csv_field(&d.name), format!("{:.5}", d.lat.unwrap()), format!("{:.5}", d.lon.unwrap()), String::new(),
            start.clone(), end.clone(), String::new(), csv_field(d.serial.as_deref().unwrap_or("")), csv_field(d.model.as_deref().unwrap_or("")),
            String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), if d.tz_known { "false" } else { "true" }.into(),
            String::new(), String::new(), String::new(), String::new(), String::new(), String::new()].join(","));
        depl.push('\n');
        rep.deployments += 1;
    }
    if rep.media == 0 { bail!("No images to export: Camtrap DP needs a time for each image and a place for each camera, and leaves out images with people."); }
    let lats: Vec<f64> = deps.iter().filter(|d| span.contains_key(&d.id)).filter_map(|d| d.lat).collect();
    let lons: Vec<f64> = deps.iter().filter(|d| span.contains_key(&d.id)).filter_map(|d| d.lon).collect();
    let (x0, x1) = (lons.iter().cloned().fold(f64::MAX, f64::min), lons.iter().cloned().fold(f64::MIN, f64::max));
    let (y0, y1) = (lats.iter().cloned().fold(f64::MAX, f64::min), lats.iter().cloned().fold(f64::MIN, f64::max));
    let lic = |scope: &str| serde_json::json!({ "name": info.license, "scope": scope });
    let schema = |t: &str| format!("https://raw.githubusercontent.com/tdwg/camtrap-dp/1.0.1/{t}-table-schema.json");
    let res = |name: &str| serde_json::json!({ "name": name, "path": format!("{name}.csv"), "profile": "tabular-data-resource", "format": "csv",
        "mediatype": "text/csv", "encoding": "utf-8", "schema": schema(name) });
    let mut contributors: Vec<serde_json::Value> = info.people.iter().map(|n| serde_json::json!({ "title": n, "role": "contributor" })).collect();
    if contributors.is_empty() { contributors.push(serde_json::json!({ "title": info.institution.clone().unwrap_or_else(|| "Wildlife Network Studio user".into()), "role": "rightsHolder" })); }
    let pkg = serde_json::json!({
        "profile": "https://raw.githubusercontent.com/tdwg/camtrap-dp/1.0.1/camtrap-dp-profile.json",
        "name": ds.to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_' && c != '.', "-"),
        "title": info.title,
        "description": format!("{} Species identified with SpeciesNet 4.0.3a (Google, Apache 2.0) on boxes from MegaDetector v6 MIT (Microsoft Pytorch-Wildlife), in Wildlife Network Studio {}; per-species thresholds fitted from expert review where reviewed. Images with people are not included.",
            info.description.clone().unwrap_or_default(), env!("CARGO_PKG_VERSION")).trim().to_string(),
        "created": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "contributors": contributors,
        "licenses": [lic("data"), lic("media")],
        "project": { "title": info.title, "samplingDesign": "opportunistic", "captureMethod": ["activityDetection"], "individualAnimals": false, "observationLevel": ["media"] },
        "spatial": { "type": "Polygon", "bbox": [x0, y0, x1, y1], "coordinates": [[[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]] },
        "temporal": { "start": t_min.as_deref().map(|t| &t[..10]), "end": t_max.as_deref().map(|t| &t[..10]) },
        "taxonomic": taxa.iter().map(|(s, r)| serde_json::json!({ "scientificName": s, "taxonRank": r })).collect::<Vec<_>>(),
        "resources": [res("deployments"), res("media"), res("observations")],
    });
    std::fs::write(dir.join("deployments.csv"), depl)?;
    std::fs::write(dir.join("media.csv"), media)?;
    std::fs::write(dir.join("observations.csv"), obs)?;
    let mut f = std::fs::File::create(dir.join("datapackage.json"))?;
    f.write_all(serde_json::to_string_pretty(&pkg)?.as_bytes())?;
    Ok(rep)
}

/// Settings of one deployment that only images use: country (ISO 3166-1 alpha-3) and US state, for geofencing.
/// Changing them clears that deployment's SpeciesNet results so the next run uses the new place.
pub fn set_country(p: &Project, dep: i64, country: Option<&str>, admin1: Option<&str>) -> Result<bool> {
    let country = country.map(|c| c.trim().to_ascii_uppercase()).filter(|c| !c.is_empty());
    if country.as_ref().is_some_and(|c| c.len() != 3 || !c.chars().all(|x| x.is_ascii_alphabetic())) { bail!("Use the three-letter country code, for example KEN, IND or USA."); }
    let admin1 = admin1.map(|c| c.trim().to_ascii_uppercase()).filter(|c| !c.is_empty() && country.as_deref() == Some("USA"));
    let before: (Option<String>, Option<String>) = p.db.query_row("SELECT country, admin1 FROM deployments WHERE id = ?1", [dep], |r| Ok((r.get(0)?, r.get(1)?))).optional()?.context("no such deployment")?;
    if before == (country.clone(), admin1.clone()) { return Ok(false); }
    p.db.execute("UPDATE deployments SET country = ?2, admin1 = ?3 WHERE id = ?1", params![dep, country, admin1])?;
    // Geofencing changes the answers, so those images run again (reviews of their boxes go with them).
    let n = p.db.execute("DELETE FROM image_models WHERE image_id IN (SELECT id FROM images WHERE deployment_id = ?1)", [dep])?;
    p.db.execute("DELETE FROM image_boxes WHERE image_id IN (SELECT id FROM images WHERE deployment_id = ?1)", [dep])?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(tag: &str) -> (Project, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("wlnw-images-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        (Project::open(&dir.join("t.wlnw")).unwrap(), dir)
    }

    #[test]
    fn accepted_thresholds_people_and_camtrapdp() {
        let (p, dir) = project("acc");
        p.db.execute_batch("INSERT INTO deployments (id, name, folder, lat, lon, tz_offset_min, tz_known) VALUES (1, 'Cam A', '/c', -1.5, 36.8, 180, 1);
            INSERT INTO species (id, scientific, common) VALUES (1, 'Panthera leo', 'lion'), (2, 'Felidae', 'cat family');
            INSERT INTO images (id, deployment_id, path, taken_local) VALUES (1, 1, '/c/a.jpg', '2026-01-02 03:04:05'), (2, 1, '/c/b.jpg', '2026-01-02 04:00:00'),
              (3, 1, '/c/c.jpg', '2026-01-02 05:00:00'), (4, 1, '/c/d.jpg', NULL), (5, 1, '/c/e.jpg', '2026-01-03 06:00:00');
            INSERT INTO image_models (image_id, model, status, processed_at) VALUES (1, 'speciesnet-4.0.3a', 'done', ''), (2, 'speciesnet-4.0.3a', 'done', ''),
              (3, 'speciesnet-4.0.3a', 'done', ''), (4, 'speciesnet-4.0.3a', 'done', ''), (5, 'speciesnet-4.0.3a', 'done', '');
            INSERT INTO image_boxes (id, image_id, model, category, conf, x, y, w, h, kind, species, label, score) VALUES
              (1, 1, 'speciesnet-4.0.3a', 'animal', 0.9, 0.1, 0.1, 0.2, 0.2, 'animal', 1, 'a;mammalia;carnivora;felidae;panthera;leo;lion', 0.95),
              (2, 1, 'speciesnet-4.0.3a', 'animal', 0.8, 0.5, 0.5, 0.2, 0.2, 'animal', 1, 'a;mammalia;carnivora;felidae;panthera;leo;lion', 0.30),
              (3, 2, 'speciesnet-4.0.3a', 'animal', 0.9, 0.1, 0.1, 0.2, 0.2, 'animal', 2, 'c;mammalia;carnivora;felidae;;;cat family', 0.9),
              (4, 2, 'speciesnet-4.0.3a', 'human', 0.9, 0.6, 0.1, 0.2, 0.5, 'human', NULL, 'h', 0.9),
              (5, 3, 'speciesnet-4.0.3a', 'vehicle', 0.9, 0.6, 0.1, 0.2, 0.5, 'vehicle', NULL, 'v', 0.9),
              (6, 4, 'speciesnet-4.0.3a', 'animal', 0.9, 0.1, 0.1, 0.2, 0.2, 'animal', 1, 'a;mammalia;carnivora;felidae;panthera;leo;lion', 0.9);").unwrap();
        // Box 2 is under the default 0.5; image 2 has a person, so its cat is left out.
        let acc = accepted(&p).unwrap();
        assert_eq!(acc.iter().map(|o| o.box_id).collect::<Vec<_>>(), vec![6, 1]); // no time sorts first
        assert_eq!(acc[1].rank, "species");
        assert_eq!(acc[1].utc.unwrap().to_string(), "2026-01-02 00:04:05");
        set_verdict(&p, 2, Some("yes")).unwrap();
        let acc = accepted(&p).unwrap();
        assert_eq!(per_image(&acc).iter().map(|(o, n)| (o.image_id, *n)).collect::<Vec<_>>(), vec![(4, 1), (1, 2)]);
        // Rejecting the person box brings image 2 back.
        set_verdict(&p, 4, Some("no")).unwrap();
        assert_eq!(accepted(&p).unwrap().len(), 4);
        set_verdict(&p, 4, Some("yes")).unwrap();
        let fc = filter_counts(&p).unwrap();
        assert_eq!((fc["animals"], fc["people"], fc["vehicles"], fc["empty"]), (3, 1, 1, 1));
        let q = queue(&p, &Query { species: Some(1), model: Some(camtrap::ID.into()), ..Default::default() }).unwrap();
        assert_eq!(q.len(), 3);
        assert_eq!(q[0].focus, Some(1));
        let people = queue(&p, &Query { show: Some("people".into()), ..Default::default() }).unwrap();
        assert_eq!((people.len(), people[0].focus), (1, Some(4)));
        let out = dir.join("dp");
        let r = camtrapdp(&p, &out).unwrap();
        assert_eq!((r.media, r.left_out_people, r.left_out_no_time_or_place, r.deployments), (3, 1, 1, 1));
        let obs = std::fs::read_to_string(out.join("observations.csv")).unwrap();
        assert!(obs.contains(",animal,,Panthera leo,1,") && obs.contains(",vehicle,") && obs.contains(",blank,"));
        assert!(!obs.contains("Felidae"));
        let media = std::fs::read_to_string(out.join("media.csv")).unwrap();
        assert!(media.contains("2026-01-02T03:04:05+03:00") && !media.contains("b.jpg"));
        let pkg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out.join("datapackage.json")).unwrap()).unwrap();
        assert_eq!(pkg["taxonomic"][0]["scientificName"], "Panthera leo");
        assert_eq!(pkg["temporal"]["start"], "2026-01-02");
        // A country change clears the deployment's results so they run again with geofencing.
        assert!(set_country(&p, 1, Some("ken"), None).unwrap());
        assert_eq!(todo(&p, camtrap::ID).unwrap(), 5);
        assert!(set_country(&p, 1, Some("Kenya"), None).is_err());
        drop(p);
        std::fs::remove_dir_all(&dir).ok();
    }
}
