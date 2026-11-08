//! Opt-in sharing of checked answers with wlnw.org, to train open regional classifiers (Settings, "Share my checked answers").
//! Each answer sent is: the species, yes or no, the Perch 2.0 embedding of the clip (1,536 numbers, not audio and not
//! reversible to audio), the recorder's place rounded to 1 degree, and the year and week. Never audio, file names, exact
//! times, project names or anything about the person. A random key kept only by this computer lets the server delete
//! everything shared with it. Only answers whose clip has a Perch embedding can be shared (Perch must have run there).
use crate::online::SITE;
use crate::perch::EMBED;
use crate::store::Project;
use anyhow::{bail, Result};
use base64::Engine as _;
use chrono::{Datelike, Duration, NaiveDateTime};
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const BATCH: usize = 200;
pub const EMBED_MODEL: &str = "perch-2.0";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Item {
    pub cid: String, pub site: String, pub species: String, pub label: u8, pub source: &'static str, pub model: String,
    pub confidence: Option<f64>, pub lat: f64, pub lon: f64, pub year: i32, pub week: u32, pub embed_model: &'static str, pub emb: String,
}

/// One answer waiting to be sent, with what marks it as sent.
pub struct Pending { pub kind: &'static str, pub reference: i64, pub item: Item }

fn hash(parts: &[&str], n: usize) -> String {
    let mut h = Sha256::new();
    for p in parts { h.update(p.as_bytes()); h.update(b"|"); }
    hex::encode(h.finalize())[..n].to_string()
}

/// A random id per project, so ids stay the same when the project file is moved and differ between projects.
fn project_id(p: &Project, key: &str) -> Result<String> {
    if let Some(id) = p.setting("share_project_id")? { return Ok(id); }
    let id = hash(&[key, &p.path.display().to_string(), &chrono::Utc::now().to_rfc3339(), &std::process::id().to_string()], 32);
    p.set_setting("share_project_id", &id)?;
    Ok(id)
}

/// "Genus species" or "Genus species subspecies": labels such as "Engine" or "Human vocal" are not shared.
pub fn is_species(name: &str) -> bool {
    // BirdNET's non-animal classes that happen to look like two-word names.
    if ["Human vocal", "Human non-vocal", "Human whistle", "Power tools"].contains(&name.trim()) { return false; }
    let w: Vec<&str> = name.split_whitespace().collect();
    (2..=3).contains(&w.len()) && w[0].chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && w.iter().all(|x| x.chars().all(|c| c.is_ascii_alphabetic() || c == '-')) && w[1..].iter().all(|x| x.chars().all(|c| !c.is_ascii_uppercase()))
}

/// Year and ISO week of a moment `offset_s` seconds into a recording that started at `start_local`.
pub fn year_week(start_local: &str, offset_s: f64) -> Option<(i32, u32)> {
    let t = NaiveDateTime::parse_from_str(start_local, "%Y-%m-%d %H:%M:%S").ok()? + Duration::milliseconds((offset_s * 1000.0) as i64);
    let w = t.iso_week();
    Some((w.year(), w.week()))
}

pub fn round_deg(x: f64) -> f64 { let r = x.round(); if r == 0.0 { 0.0 } else { r } }

/// Answers not yet shared, at most `limit`: reviewed detections (yes or no) and named examples, each with its Perch embedding.
pub fn pending(p: &Project, key: &str, limit: usize) -> Result<Vec<Pending>> {
    let pid = project_id(p, key)?;
    let mut out: Vec<Pending> = Vec::new();
    let mut rows: Vec<(&'static str, i64, String, u8, String, Option<f64>, i64, f64, f64, f64, String, i64)> = Vec::new();
    {
        // Reviewed detections: the Perch window around the middle of the detection.
        let mut st = p.db.prepare(
            "SELECT d.id, s.scientific, CASE r.verdict WHEN 'yes' THEN 1 ELSE 0 END, d.model, d.confidence, dep.id, dep.lat, dep.lon,
                    (d.start_s + d.end_s) / 2.0, f.start_local,
                    (SELECT w.row FROM windows w WHERE w.file_id = d.file_id AND w.model = ?1 AND w.start_s <= (d.start_s + d.end_s) / 2.0
                       AND w.end_s > (d.start_s + d.end_s) / 2.0 ORDER BY w.start_s LIMIT 1)
             FROM reviews r JOIN detections d ON d.id = r.detection_id JOIN species s ON s.id = d.species
             JOIN files f ON f.id = d.file_id JOIN deployments dep ON dep.id = f.deployment_id
             WHERE r.verdict IN ('yes', 'no') AND dep.lat IS NOT NULL AND dep.lon IS NOT NULL AND f.start_local IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM shared x WHERE x.kind = 'review' AND x.ref = d.id)
             ORDER BY d.id")?;
        let q = st.query_map(params![EMBED_MODEL], |r| Ok(("review", r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? as u8, r.get::<_, String>(3)?,
            r.get::<_, Option<f64>>(4)?, r.get::<_, i64>(5)?, r.get::<_, f64>(6)?, r.get::<_, f64>(7)?, r.get::<_, f64>(8)?, r.get::<_, String>(9)?, r.get::<_, Option<i64>>(10)?)))?;
        for r in q { let r = r?; if let Some(row) = r.11 { rows.push((r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10, row)); } }
    }
    {
        // Sounds the user named (present or absent): the window is the example itself.
        let mut st = p.db.prepare(
            "SELECT w.id, s.scientific, x.present, w.model, dep.id, dep.lat, dep.lon, (w.start_s + w.end_s) / 2.0, f.start_local, w.row
             FROM examples x JOIN windows w ON w.id = x.window_id JOIN species s ON s.id = x.species
             JOIN files f ON f.id = w.file_id JOIN deployments dep ON dep.id = f.deployment_id
             WHERE w.model = ?1 AND dep.lat IS NOT NULL AND dep.lon IS NOT NULL AND f.start_local IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM shared y WHERE y.kind = 'example' AND y.ref = w.id)
             ORDER BY w.id")?;
        let q = st.query_map(params![EMBED_MODEL], |r| Ok(("example", r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? as u8, r.get::<_, String>(3)?,
            None, r.get::<_, i64>(4)?, r.get::<_, f64>(5)?, r.get::<_, f64>(6)?, r.get::<_, f64>(7)?, r.get::<_, String>(8)?, r.get::<_, i64>(9)?)))?;
        for r in q { rows.push(r?); }
    }
    let rows: Vec<_> = rows.into_iter().filter(|r| is_species(&r.2)).take(limit).collect();
    if rows.is_empty() { return Ok(out); }
    let embs = p.read_embeddings(&rows.iter().map(|r| r.11).collect::<Vec<_>>(), EMBED)?;
    for (r, e) in rows.into_iter().zip(embs) {
        let (kind, reference, species, label, model, confidence, dep, lat, lon, mid, start, _) = r;
        let Some((year, week)) = year_week(&start, mid) else { continue };
        let bytes: Vec<u8> = e.iter().flat_map(|v| half::f16::from_f32(*v).to_le_bytes()).collect();
        out.push(Pending { kind, reference, item: Item {
            cid: hash(&[key, &pid, kind, &reference.to_string()], 32), site: hash(&[key, &pid, "site", &dep.to_string()], 16),
            species, label, source: kind, model, confidence, lat: round_deg(lat), lon: round_deg(lon), year, week,
            embed_model: EMBED_MODEL, emb: base64::engine::general_purpose::STANDARD.encode(bytes),
        } });
    }
    Ok(out)
}

pub fn mark(p: &Project, sent: &[Pending]) -> Result<()> {
    let tx = p.db.unchecked_transaction()?;
    { let mut st = tx.prepare("INSERT OR IGNORE INTO shared (kind, ref, at) VALUES (?1, ?2, datetime('now'))")?;
      for s in sent { st.execute(params![s.kind, s.reference])?; } }
    tx.commit()?;
    Ok(())
}

/// How many answers this project has shared, and how many could still be.
pub fn counts(p: &Project, key: &str) -> Result<(i64, usize)> {
    let shared: i64 = p.db.query_row("SELECT COUNT(*) FROM shared", [], |r| r.get(0))?;
    Ok((shared, pending(p, key, usize::MAX)?.len()))
}

pub enum Outcome { Sent(usize), Paused(String) }

/// Sends everything pending, 200 at a time. `post` does the HTTP call (tests pass a fake).
pub fn sync(p: &Project, key: &str, post: &dyn Fn(&serde_json::Value) -> Result<(u16, serde_json::Value)>) -> Result<Outcome> {
    let mut total = 0;
    loop {
        let batch = pending(p, key, BATCH)?;
        if batch.is_empty() { return Ok(Outcome::Sent(total)); }
        let body = serde_json::json!({ "key": key, "app": env!("CARGO_PKG_VERSION"), "items": batch.iter().map(|b| &b.item).collect::<Vec<_>>() });
        let (code, res) = post(&body)?;
        match code {
            200 => { mark(p, &batch)?; total += batch.len(); }
            429 | 503 => return Ok(Outcome::Paused(res["error"].as_str().unwrap_or("Sharing is paused for now.").to_string())),
            _ => bail!("wlnw.org answered {code}: {}", res["error"].as_str().unwrap_or("")),
        }
        if batch.len() < BATCH { return Ok(Outcome::Sent(total)); }
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(60)).user_agent(concat!("WildlifeNetworkStudio/", env!("CARGO_PKG_VERSION"))).build()
}

pub fn http_post(body: &serde_json::Value) -> Result<(u16, serde_json::Value)> {
    match agent().post(&format!("{SITE}/api/v1/shared-answers")).send_json(body) {
        Ok(r) => Ok((r.status(), r.into_json().unwrap_or_default())),
        Err(ureq::Error::Status(code, r)) => Ok((code, r.into_json().unwrap_or_default())),
        Err(e) => Err(e.into()),
    }
}

/// Deletes everything shared with this key on wlnw.org; returns how many answers were removed.
pub fn delete_all(key: &str) -> Result<i64> {
    let r = agent().delete(&format!("{SITE}/api/v1/shared-answers")).send_json(serde_json::json!({ "key": key }))?;
    let v: serde_json::Value = r.into_json()?;
    Ok(v["deleted"].as_i64().unwrap_or(0))
}

/// Forgets which answers were shared, so turning sharing back on after a delete sends them again.
pub fn forget(p: &Project) -> Result<()> { p.db.execute("DELETE FROM shared", [])?; Ok(()) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert!(is_species("Upupa epops"));
        assert!(is_species("Motacilla alba yarrellii"));
        assert!(!is_species("Engine"));
        assert!(!is_species("Human vocal"));
        assert!(!is_species("Upupa Epops"));
        assert_eq!(year_week("2025-12-31 23:00:00", 7200.0), Some((2026, 1)));
        assert_eq!(year_week("2025-11-20 06:00:00", 10.0), Some((2025, 47)));
        assert_eq!(round_deg(-0.4), 0.0);
        assert_eq!(round_deg(12.6), 13.0);
    }

    #[test]
    fn pending_and_sync() {
        let dir = std::env::temp_dir().join(format!("wlnw-share-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = Project::open(&dir.join("t.wlnw")).unwrap();
        p.db.execute_batch("INSERT INTO deployments (id, name, folder, lat, lon) VALUES (1, 'N', '/x', 12.34, 77.6);
            INSERT INTO files (id, deployment_id, path, start_local) VALUES (1, 1, '/x/a.wav', '2025-11-20 06:00:00');
            INSERT INTO species (id, scientific) VALUES (1, 'Upupa epops'), (2, 'Engine');
            INSERT INTO detections (id, file_id, model, start_s, end_s, species, confidence, in_range) VALUES
              (1, 1, 'birdnet-2.4', 0, 3, 1, 0.8, 1), (2, 1, 'birdnet-2.4', 5, 8, 1, 0.3, 1), (3, 1, 'birdnet-2.4', 0, 3, 2, 0.9, 1), (4, 1, 'birdnet-2.4', 20, 23, 1, 0.5, 1);
            INSERT INTO reviews VALUES (1, 'yes', '2026-11-08'), (2, 'no', '2026-11-08'), (3, 'yes', '2026-11-08'), (4, 'unsure', '2026-11-08');
            INSERT INTO windows (id, file_id, model, start_s, end_s, row) VALUES (1, 1, 'perch-2.0', 0, 5, 0), (2, 1, 'perch-2.0', 5, 10, 1);").unwrap();
        // Two embeddings in the .emb file.
        let emb: Vec<u8> = (0..2 * EMBED).flat_map(|i| half::f16::from_f32(i as f32 / 1000.0).to_le_bytes()).collect();
        std::fs::write(dir.join("t.wlnw.emb"), emb).unwrap();
        let key = "a".repeat(64);
        let got = pending(&p, &key, 100).unwrap();
        // The engine noise label and the unsure answer are left out.
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].item.label, got[1].item.label), (1, 0));
        assert_eq!((got[0].item.lat, got[0].item.lon, got[0].item.year, got[0].item.week), (12.0, 78.0, 2025, 47));
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&got[0].item.emb).unwrap().len(), EMBED * 2);
        assert_ne!(got[0].item.cid, got[1].item.cid);
        let sent = std::cell::Cell::new(0);
        let r = sync(&p, &key, &|b| { sent.set(sent.get() + b["items"].as_array().unwrap().len()); Ok((200, serde_json::json!({}))) }).unwrap();
        assert!(matches!(r, Outcome::Sent(2)));
        assert_eq!(sent.get(), 2);
        assert_eq!(pending(&p, &key, 100).unwrap().len(), 0);
        forget(&p).unwrap();
        assert!(matches!(sync(&p, &key, &|_| Ok((503, serde_json::json!({"error": "paused"})))).unwrap(), Outcome::Paused(_)));
        drop(p);
        std::fs::remove_dir_all(&dir).ok();
    }
}
