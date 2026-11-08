//! A project is one SQLite file: the recordings found, the deployments they belong to, every detection (with the model
//! that made it), and the reviews. Embeddings (Perch) live next to it in `<project>.emb`, 1,536 half-floats per window.
//! Each recording is committed per model when finished, so a crash or a closed laptop loses at most the file in progress.
use anyhow::{bail, Result};
use chrono::{Duration, NaiveDateTime};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub struct Project { pub db: Connection, pub path: PathBuf }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub id: i64, pub name: String, pub folder: String, pub serial: Option<String>, pub model: Option<String>,
    pub lat: Option<f64>, pub lon: Option<f64>,
    /// Offset from UTC in minutes of the times in the file names / metadata.
    pub tz_offset_min: i32,
    pub tz_known: bool,
    pub files: i64, pub done: i64, pub hours: f64,
    /// Camera traps: images found, and the country (ISO 3166-1 alpha-3) and US state SpeciesNet's geofencing uses.
    #[serde(default)] pub images: i64,
    #[serde(default)] pub country: Option<String>,
    #[serde(default)] pub admin1: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detection {
    pub id: i64, pub file_id: i64, pub path: String, pub deployment_id: i64, pub start_s: f64, pub end_s: f64,
    /// Species id (table species).
    pub label: i64, pub scientific: String, pub common: String, pub model: String, pub confidence: f64, pub in_range: bool,
    /// Start of the detection in UTC, when the recording's time is known.
    pub utc: Option<NaiveDateTime>, pub lat: Option<f64>, pub lon: Option<f64>, pub site: String,
    pub verdict: Option<String>,
    /// Call models (BatDetect2): how sure the model is that this is a bat call at all; `confidence` is the species score.
    #[serde(default)]
    pub detection_score: Option<f64>,
}

const SCHEMA_VERSION: &str = "2";
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS deployments (
  id INTEGER PRIMARY KEY, name TEXT NOT NULL, folder TEXT NOT NULL, serial TEXT, model TEXT,
  lat REAL, lon REAL, tz_offset_min INTEGER NOT NULL DEFAULT 0, tz_known INTEGER NOT NULL DEFAULT 0,
  UNIQUE (folder, serial));
CREATE TABLE IF NOT EXISTS files (
  id INTEGER PRIMARY KEY, deployment_id INTEGER NOT NULL REFERENCES deployments(id), path TEXT NOT NULL UNIQUE,
  start_local TEXT, time_source TEXT, bytes INTEGER, duration_s REAL);
-- One row per recording and model: 'done' or 'error'. A recording with no row for a model is still to do.
CREATE TABLE IF NOT EXISTS file_models (file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE, model TEXT NOT NULL,
  status TEXT NOT NULL, error TEXT, processed_at TEXT NOT NULL, PRIMARY KEY (file_id, model));
CREATE TABLE IF NOT EXISTS species (id INTEGER PRIMARY KEY, scientific TEXT NOT NULL UNIQUE, common TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS detections (
  id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE, model TEXT NOT NULL,
  start_s REAL NOT NULL, end_s REAL NOT NULL, species INTEGER NOT NULL REFERENCES species(id), confidence REAL NOT NULL, in_range INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS detections_species ON detections (species, confidence);
CREATE INDEX IF NOT EXISTS detections_file ON detections (file_id, model);
CREATE TABLE IF NOT EXISTS reviews (detection_id INTEGER PRIMARY KEY REFERENCES detections(id) ON DELETE CASCADE,
  verdict TEXT NOT NULL CHECK (verdict IN ('yes','no','unsure')), at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS thresholds (species INTEGER NOT NULL, model TEXT NOT NULL, threshold REAL NOT NULL, precision REAL,
  reviewed INTEGER NOT NULL, source TEXT NOT NULL, PRIMARY KEY (species, model));
-- Windows with an embedding: row = position in <project>.emb.
CREATE TABLE IF NOT EXISTS windows (id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  model TEXT NOT NULL, start_s REAL NOT NULL, end_s REAL NOT NULL, row INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS windows_file ON windows (file_id, start_s);
-- Examples named by the user for the Wildlife Network classifier: a window and the species heard in it (or 'absent').
-- What was done and when: runs, folders added, training, exports, publishes (history.rs).
CREATE TABLE IF NOT EXISTS history (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, started_at TEXT NOT NULL, finished_at TEXT,
  status TEXT NOT NULL, summary TEXT NOT NULL DEFAULT '', detail TEXT NOT NULL DEFAULT '{}');
CREATE TABLE IF NOT EXISTS examples (window_id INTEGER NOT NULL REFERENCES windows(id) ON DELETE CASCADE, species INTEGER NOT NULL REFERENCES species(id),
  present INTEGER NOT NULL, at TEXT NOT NULL, PRIMARY KEY (window_id, species));
-- Another reviewer's answers loaded from a .wlnr file (agreement.rs), kept apart from this project's own reviews.
-- Settling a disagreement writes reviews and keeps the first answer in mine_before, so agreement figures do not change.
CREATE TABLE IF NOT EXISTS reviews_other (reviewer TEXT NOT NULL, detection_id INTEGER NOT NULL REFERENCES detections(id) ON DELETE CASCADE,
  verdict TEXT NOT NULL CHECK (verdict IN ('yes','no','unsure')), at TEXT NOT NULL, mine_before TEXT, settled_at TEXT, PRIMARY KEY (reviewer, detection_id));
-- Answers from wlnw.org's public API (online.rs), kept 30 days so the project works offline and carries them.
CREATE TABLE IF NOT EXISTS web_cache (key TEXT PRIMARY KEY, json TEXT NOT NULL, fetched_at TEXT NOT NULL);
"#;

impl Project {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
        let old: bool = db.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'detections')", [], |r| r.get(0))?
            && !db.query_row("SELECT EXISTS (SELECT 1 FROM pragma_table_info('detections') WHERE name = 'model')", [], |r| r.get::<_, bool>(0))?;
        if old { bail!("This project was made by an earlier test version of Studio. Please create a new project and add the recordings again."); }
        db.execute_batch(SCHEMA)?;
        db.execute_batch(crate::images::SCHEMA)?;
        // Columns added after schema 2, appended to projects made before them (nullable, so old rows simply have none).
        for (table, col, ty) in [("detections", "detection_score", "REAL"), ("files", "sample_rate", "INTEGER"), ("deployments", "country", "TEXT"), ("deployments", "admin1", "TEXT")] {
            let has: bool = db.query_row(&format!("SELECT EXISTS (SELECT 1 FROM pragma_table_info('{table}') WHERE name = '{col}')"), [], |r| r.get(0))?;
            if !has { db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {ty}"))?; }
        }
        let p = Project { db, path: path.to_path_buf() };
        p.set_setting("schema", SCHEMA_VERSION)?;
        Ok(p)
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.db.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.db.execute("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value", params![key, value])?;
        Ok(())
    }
    /// Models chosen for this project; empty means "the default for each recorder's region".
    pub fn chosen_models(&self) -> Vec<String> {
        self.setting("models").ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn deployments(&self) -> Result<Vec<Deployment>> {
        let mut st = self.db.prepare(
            "SELECT d.id, d.name, d.folder, d.serial, d.model, d.lat, d.lon, d.tz_offset_min, d.tz_known,
                    COUNT(f.id), COUNT(f.id) FILTER (WHERE f.duration_s IS NOT NULL), COALESCE(SUM(f.duration_s), 0) / 3600.0,
                    (SELECT COUNT(*) FROM images i WHERE i.deployment_id = d.id), d.country, d.admin1
             FROM deployments d LEFT JOIN files f ON f.deployment_id = d.id GROUP BY d.id ORDER BY d.name")?;
        let rows = st.query_map([], |r| Ok(Deployment {
            id: r.get(0)?, name: r.get(1)?, folder: r.get(2)?, serial: r.get(3)?, model: r.get(4)?, lat: r.get(5)?, lon: r.get(6)?,
            tz_offset_min: r.get(7)?, tz_known: r.get::<_, i64>(8)? == 1, files: r.get(9)?, done: r.get(10)?, hours: r.get(11)?,
            images: r.get(12)?, country: r.get(13)?, admin1: r.get(14)?,
        }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn update_deployment(&self, id: i64, name: &str, lat: Option<f64>, lon: Option<f64>, tz_offset_min: i32) -> Result<()> {
        self.db.execute("UPDATE deployments SET name = ?2, lat = ?3, lon = ?4, tz_offset_min = ?5, tz_known = 1 WHERE id = ?1",
            params![id, name, lat, lon, tz_offset_min])?;
        Ok(())
    }

    /// Species ids for a model's labels (created on first use), in label order. A better common name replaces an empty one.
    pub fn species_ids(&self, labels: &[crate::models::Label]) -> Result<Vec<i64>> {
        let tx = self.db.unchecked_transaction()?;
        let mut out = Vec::with_capacity(labels.len());
        {
            let mut ins = tx.prepare("INSERT INTO species (scientific, common) VALUES (?1, ?2)
                ON CONFLICT (scientific) DO UPDATE SET common = CASE WHEN species.common = '' THEN excluded.common ELSE species.common END RETURNING id")?;
            for l in labels { out.push(ins.query_row(params![l.scientific, l.common], |r| r.get(0))?); }
        }
        tx.commit()?;
        Ok(out)
    }
    pub fn species_id(&self, scientific: &str, common: &str) -> Result<i64> {
        Ok(self.db.query_row("INSERT INTO species (scientific, common) VALUES (?1, ?2)
            ON CONFLICT (scientific) DO UPDATE SET common = CASE WHEN species.common = '' THEN excluded.common ELSE species.common END RETURNING id",
            params![scientific.trim(), common.trim()], |r| r.get(0))?)
    }

    /// Detections joined with their recording, deployment, species and review. `filter` is a SQL condition on d, s (species), r (reviews), f, p.
    pub fn detections(&self, filter: &str, args: &[&dyn rusqlite::ToSql], limit: i64) -> Result<Vec<Detection>> {
        let sql = format!(
            "SELECT d.id, d.file_id, f.path, f.deployment_id, d.start_s, d.end_s, d.species, s.scientific, s.common, d.model, d.confidence, d.in_range,
                    f.start_local, p.tz_offset_min, p.lat, p.lon, p.name, r.verdict, d.detection_score
             FROM detections d JOIN files f ON f.id = d.file_id JOIN deployments p ON p.id = f.deployment_id
             JOIN species s ON s.id = d.species LEFT JOIN reviews r ON r.detection_id = d.id
             WHERE {filter} ORDER BY f.start_local, f.path, d.start_s LIMIT {limit}");
        let mut st = self.db.prepare(&sql)?;
        let rows = st.query_map(args, |r| {
            let start_local: Option<String> = r.get(12)?;
            let tz: i32 = r.get(13)?;
            let start_s: f64 = r.get(4)?;
            let utc = start_local.and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok())
                .map(|t| t - Duration::minutes(tz as i64) + Duration::milliseconds((start_s * 1000.0) as i64));
            Ok(Detection {
                id: r.get(0)?, file_id: r.get(1)?, path: r.get(2)?, deployment_id: r.get(3)?, start_s, end_s: r.get(5)?, label: r.get(6)?,
                scientific: r.get(7)?, common: r.get(8)?, model: r.get(9)?, confidence: r.get(10)?, in_range: r.get::<_, i64>(11)? == 1,
                utc, lat: r.get(14)?, lon: r.get(15)?, site: r.get(16)?, verdict: r.get(17)?, detection_score: r.get(18)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Threshold for a species from one model: fitted from reviews when there are enough, else the project's default for that model.
    pub fn threshold(&self, species: i64, model: &str) -> f64 {
        self.db.query_row("SELECT threshold FROM thresholds WHERE species = ?1 AND model = ?2", params![species, model], |r| r.get(0)).optional().ok().flatten()
            .unwrap_or_else(|| self.default_threshold(model))
    }
    pub fn default_threshold(&self, model: &str) -> f64 {
        self.setting(&format!("default_threshold:{model}")).ok().flatten().or_else(|| self.setting("default_threshold").ok().flatten())
            .and_then(|v| v.parse().ok()).unwrap_or(match model { "birdnet-2.4" => 0.5, _ => 0.5 })
    }

    /// Detections to publish or export: in range, at or above their species' threshold for their model, not rejected; a "yes" is always kept.
    /// When two models found the same species at overlapping times in one recording, only one is kept (a reviewed one, else the most confident).
    pub fn accepted(&self) -> Result<Vec<Detection>> {
        let all = self.detections("d.in_range = 1 AND (r.verdict IS NULL OR r.verdict <> 'no')", &[], i64::MAX)?;
        let mut cache = HashMap::new();
        let mut kept: Vec<Detection> = all.into_iter().filter(|d| d.verdict.as_deref() == Some("yes")
            || d.confidence >= *cache.entry((d.label, d.model.clone())).or_insert_with(|| self.threshold(d.label, &d.model))).collect();
        kept.sort_by(|a, b| (a.file_id, a.label).cmp(&(b.file_id, b.label)).then(a.start_s.total_cmp(&b.start_s)));
        let mut out: Vec<Detection> = Vec::with_capacity(kept.len());
        for d in kept {
            if let Some(last) = out.last_mut() {
                if last.file_id == d.file_id && last.label == d.label && d.start_s < last.end_s {
                    let better = (d.verdict.is_some() as u8, d.confidence) > (last.verdict.is_some() as u8, last.confidence);
                    if better { *last = d; }
                    continue;
                }
            }
            out.push(d);
        }
        out.sort_by(|a, b| (&a.utc, &a.path).cmp(&(&b.utc, &b.path)).then(a.start_s.total_cmp(&b.start_s)));
        Ok(out)
    }

    fn emb_path(&self) -> PathBuf { let mut p = self.path.clone().into_os_string(); p.push(".emb"); PathBuf::from(p) }

    /// Appends embeddings (as half floats) and returns their row numbers.
    pub fn append_embeddings(&self, rows: &[Vec<f32>]) -> Result<Vec<i64>> {
        let dim = rows.first().map_or(0, |r| r.len());
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.emb_path())?;
        let start = f.metadata()?.len() as i64 / (dim.max(1) as i64 * 2);
        let mut buf = Vec::with_capacity(rows.len() * dim * 2);
        for r in rows { for &v in r { buf.extend_from_slice(&half::f16::from_f32(v).to_le_bytes()); } }
        f.write_all(&buf)?;
        f.sync_data()?;
        Ok((0..rows.len() as i64).map(|i| start + i).collect())
    }

    /// Reads embeddings by row number.
    pub fn read_embeddings(&self, rows: &[i64], dim: usize) -> Result<Vec<Vec<f32>>> {
        let mut f = std::fs::File::open(self.emb_path())?;
        let mut out = Vec::with_capacity(rows.len());
        let mut b = vec![0u8; dim * 2];
        for &r in rows {
            f.seek(SeekFrom::Start(r as u64 * dim as u64 * 2))?;
            f.read_exact(&mut b)?;
            out.push(b.chunks_exact(2).map(|c| half::f16::from_le_bytes([c[0], c[1]]).to_f32()).collect());
        }
        Ok(out)
    }

    /// Every embedding in the file, in row order (for scoring and similarity search; about 3 KB per 5 s window).
    pub fn all_embeddings(&self, dim: usize) -> Result<Vec<Vec<f32>>> {
        let Ok(mut f) = std::fs::File::open(self.emb_path()) else { return Ok(vec![]) };
        let mut b = Vec::new();
        f.read_to_end(&mut b)?;
        Ok(b.chunks_exact(dim * 2).map(|row| row.chunks_exact(2).map(|c| half::f16::from_le_bytes([c[0], c[1]]).to_f32()).collect()).collect())
    }
}
