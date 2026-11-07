//! Wildlife Network Classifier (WNC): small per-species classifiers trained on Perch 2.0 embeddings from what reviewers
//! told us. It learns local species and sounds that BirdNET or Perch get wrong or do not know at all:
//! - every "yes" on a detection (any model) is a positive example for that species, every "no" a negative one;
//! - a reviewer can also name any clip ("this is X", or "X is not here") and ask for more like it;
//! - a trained classifier can be saved as a .wnc pack and shared, for example one for a region.
//! Each species gets a logistic regression (L2) over the 1,536 embedding numbers; positives of other species and random
//! windows of the same project are negatives. The model only runs where Perch has run (it needs the embeddings).
use crate::perch::EMBED;
use crate::store::Project;
use anyhow::{bail, Context, Result};
use base64::Engine as _;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;

pub const MODEL_ID: &str = "wnc";
pub const MIN_POSITIVES: usize = 3;
const BACKGROUND_PER_POSITIVE: usize = 5;
const EPOCHS: usize = 400;
const L2: f32 = 1e-3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WncClass {
    pub scientific: String, pub common: String, pub positives: usize, pub negatives: usize,
    /// Share of held-out examples (5-fold) classified right at the 0.5 threshold; None when there were too few.
    pub cv_accuracy: Option<f64>,
    pub weights: String, pub bias: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WncPack { pub format: String, pub name: String, pub trained_at: String, pub embedding: String, pub classes: Vec<WncClass> }

#[derive(Debug, Serialize)]
pub struct TrainReport { pub classes: Vec<(String, String, usize, usize, Option<f64>)>, pub skipped: Vec<(String, usize)>, pub detections: usize }

fn sigmoid(z: f32) -> f32 { 1.0 / (1.0 + (-z).exp()) }
fn dot(a: &[f32], b: &[f32]) -> f32 { a.iter().zip(b).map(|(x, y)| x * y).sum() }

fn fit(xs: &[&Vec<f32>], ys: &[f32]) -> (Vec<f32>, f32) {
    let d = xs.first().map_or(EMBED, |x| x.len());
    let (mut w, mut b) = (vec![0f32; d], 0f32);
    // Balance the classes so a few positives are not drowned by background.
    let pos = ys.iter().filter(|&&y| y > 0.5).count().max(1) as f32;
    let neg = (ys.len() as f32 - pos).max(1.0);
    let wt: Vec<f32> = ys.iter().map(|&y| if y > 0.5 { 0.5 / pos } else { 0.5 / neg }).collect();
    let lr = 2.0;
    for _ in 0..EPOCHS {
        let mut gw = vec![0f32; d];
        let mut gb = 0f32;
        for ((x, &y), &k) in xs.iter().zip(ys).zip(&wt) {
            let e = (sigmoid(dot(&w, x) + b) - y) * k;
            for (g, v) in gw.iter_mut().zip(x.iter()) { *g += e * v; }
            gb += e;
        }
        for (wi, g) in w.iter_mut().zip(&gw) { *wi -= lr * (g + L2 * *wi); }
        b -= lr * gb;
    }
    (w, b)
}

fn cv(xs: &[&Vec<f32>], ys: &[f32]) -> Option<f64> {
    if xs.len() < 10 { return None; }
    let (mut right, mut n) = (0, 0);
    for k in 0..5 {
        let (tr_x, tr_y): (Vec<&Vec<f32>>, Vec<f32>) = xs.iter().zip(ys).enumerate().filter(|(i, _)| i % 5 != k).map(|(_, (x, y))| (*x, *y)).unzip();
        if !tr_y.iter().any(|&y| y > 0.5) { continue; }
        let (w, b) = fit(&tr_x, &tr_y);
        for (i, (x, &y)) in xs.iter().zip(ys).enumerate() { if i % 5 == k { n += 1; if (sigmoid(dot(&w, x) + b) >= 0.5) == (y > 0.5) { right += 1; } } }
    }
    (n > 0).then(|| right as f64 / n as f64)
}

/// Windows (Perch) overlapping a time span of a file.
fn windows_at(p: &Project, file: i64, start: f64, end: f64) -> Result<Vec<(i64, i64)>> {
    let mut st = p.db.prepare_cached("SELECT id, row FROM windows WHERE file_id = ?1 AND model = 'perch-2.0' AND start_s < ?3 AND end_s > ?2")?;
    let rows = st.query_map(params![file, start, end], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Trains one classifier per species with at least MIN_POSITIVES examples, stores it in the project, and scores every window.
pub fn train(p: &Project) -> Result<TrainReport> {
    let n_windows: i64 = p.db.query_row("SELECT COUNT(*) FROM windows WHERE model = 'perch-2.0'", [], |r| r.get(0))?;
    if n_windows == 0 { bail!("The Wildlife Network classifier learns from Perch: process some recordings with Perch first."); }
    // Positive and negative window rows per species.
    let mut pos: HashMap<i64, HashSet<i64>> = HashMap::new();
    let mut neg: HashMap<i64, HashSet<i64>> = HashMap::new();
    let mut st = p.db.prepare("SELECT d.file_id, d.start_s, d.end_s, d.species, r.verdict FROM detections d JOIN reviews r ON r.detection_id = d.id WHERE r.verdict IN ('yes', 'no')")?;
    let reviewed: Vec<(i64, f64, f64, i64, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<_, _>>()?;
    for (file, s, e, sp, v) in reviewed {
        for (_, row) in windows_at(p, file, s, e)? { if v == "yes" { pos.entry(sp).or_default().insert(row); } else { neg.entry(sp).or_default().insert(row); } }
    }
    let mut st = p.db.prepare("SELECT w.row, x.species, x.present FROM examples x JOIN windows w ON w.id = x.window_id")?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
        let (row, sp, present) = r?;
        if present == 1 { pos.entry(sp).or_default().insert(row); } else { neg.entry(sp).or_default().insert(row); }
    }
    let names: HashMap<i64, (String, String)> = {
        let mut st = p.db.prepare("SELECT id, scientific, common FROM species")?;
        let v = st.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?.collect::<Result<_, _>>()?;
        v
    };
    // Background: a spread of windows from the whole project.
    let bg_rows: Vec<i64> = {
        let want = (pos.values().map(|s| s.len()).max().unwrap_or(0) * BACKGROUND_PER_POSITIVE).clamp(50, 3000) as i64;
        let mut st = p.db.prepare("SELECT row FROM windows WHERE model = 'perch-2.0' ORDER BY (id * 2654435761) % 1000003 LIMIT ?1")?;
        let v = st.query_map([want], |r| r.get(0))?.collect::<Result<_, _>>()?;
        v
    };
    let all_rows: Vec<i64> = { let mut v: Vec<i64> = pos.values().chain(neg.values()).flatten().copied().chain(bg_rows.iter().copied()).collect(); v.sort(); v.dedup(); v };
    let emb: HashMap<i64, Vec<f32>> = all_rows.iter().copied().zip(p.read_embeddings(&all_rows, EMBED)?).collect();
    let mut classes = Vec::new();
    let mut report = TrainReport { classes: vec![], skipped: vec![], detections: 0 };
    let mut species_ids: Vec<i64> = pos.keys().copied().collect();
    species_ids.sort();
    for sp in species_ids {
        let positives = &pos[&sp];
        let (sci, com) = names.get(&sp).cloned().unwrap_or_default();
        if positives.len() < MIN_POSITIVES { report.skipped.push((sci, positives.len())); continue; }
        let mut negatives: HashSet<i64> = neg.get(&sp).cloned().unwrap_or_default();
        for (other, rows) in &pos { if *other != sp { negatives.extend(rows.iter().filter(|r| !positives.contains(r))); } }
        negatives.extend(bg_rows.iter().filter(|r| !positives.contains(r)));
        let mut xs: Vec<&Vec<f32>> = Vec::new();
        let mut ys: Vec<f32> = Vec::new();
        for r in positives { xs.push(&emb[r]); ys.push(1.0); }
        for r in &negatives { xs.push(&emb[r]); ys.push(0.0); }
        // Interleave so the 5 folds each hold positives.
        let mut idx: Vec<usize> = (0..xs.len()).collect();
        idx.sort_by_key(|&i| (i * 2654435761) % 1000003);
        let (xs, ys): (Vec<&Vec<f32>>, Vec<f32>) = idx.iter().map(|&i| (xs[i], ys[i])).unzip();
        let acc = cv(&xs, &ys);
        let (w, b) = fit(&xs, &ys);
        let bytes: Vec<u8> = w.iter().flat_map(|v| v.to_le_bytes()).collect();
        report.classes.push((sci.clone(), com.clone(), positives.len(), negatives.len(), acc));
        classes.push(WncClass { scientific: sci, common: com, positives: positives.len(), negatives: negatives.len(), cv_accuracy: acc,
            weights: base64::engine::general_purpose::STANDARD.encode(bytes), bias: b });
    }
    let pack = WncPack { format: "wnc/1".into(), name: "Wildlife Network Classifier".into(), trained_at: chrono::Utc::now().to_rfc3339(), embedding: "perch-2.0".into(), classes };
    p.set_setting("wnc", &serde_json::to_string(&pack)?)?;
    report.detections = score(p, &pack)?;
    Ok(report)
}

pub fn pack(p: &Project) -> Option<WncPack> { p.setting("wnc").ok().flatten().and_then(|s| serde_json::from_str(&s).ok()) }

/// Adds the classes of a shared pack to the project's own (a project's own class wins when both have a species), then scores.
pub fn import(p: &Project, json: &str) -> Result<usize> {
    let incoming: WncPack = serde_json::from_str(json).context("not a Wildlife Network classifier file")?;
    if incoming.format != "wnc/1" || incoming.embedding != "perch-2.0" { bail!("this classifier file is for another version"); }
    let mut mine = pack(p).unwrap_or(WncPack { format: "wnc/1".into(), name: "Wildlife Network Classifier".into(), trained_at: chrono::Utc::now().to_rfc3339(), embedding: "perch-2.0".into(), classes: vec![] });
    let have: HashSet<String> = mine.classes.iter().map(|c| c.scientific.clone()).collect();
    mine.classes.extend(incoming.classes.into_iter().filter(|c| !have.contains(&c.scientific)));
    p.set_setting("wnc", &serde_json::to_string(&mine)?)?;
    score(p, &mine)
}

/// Scores every Perch window with the pack and stores detections with model "wnc" at 0.5 or more. Reviews of earlier
/// WNC detections are kept (matched by recording, time and species).
pub fn score(p: &Project, pack: &WncPack) -> Result<usize> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut heads: Vec<(i64, Vec<f32>, f32)> = Vec::new();
    for c in &pack.classes {
        let bytes = b64.decode(&c.weights)?;
        let w: Vec<f32> = bytes.chunks_exact(4).map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]])).collect();
        heads.push((p.species_id(&c.scientific, &c.common)?, w, c.bias));
    }
    let wins: Vec<(i64, f64, f64, i64)> = {
        let mut st = p.db.prepare("SELECT file_id, start_s, end_s, row FROM windows WHERE model = 'perch-2.0' ORDER BY row")?;
        let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        v
    };
    let kept: HashMap<(i64, i64, i64), String> = {
        let mut st = p.db.prepare("SELECT d.file_id, CAST(d.start_s * 10 AS INTEGER), d.species, r.verdict FROM detections d JOIN reviews r ON r.detection_id = d.id WHERE d.model = ?1")?;
        let v = st.query_map([MODEL_ID], |r| Ok(((r.get(0)?, r.get(1)?, r.get(2)?), r.get(3)?)))?.collect::<Result<_, _>>()?;
        v
    };
    let by_row: HashMap<i64, (i64, f64, f64)> = wins.iter().map(|&(f, s, e, r)| (r, (f, s, e))).collect();
    let mut found = Vec::new();
    stream(p, |row, x| {
        if let Some(&(f, s, e)) = by_row.get(&row) {
            for (sp, w, b) in &heads { let c = sigmoid(dot(w, x) + b); if c >= 0.5 { found.push((f, s, e, *sp, c)); } }
        }
    })?;
    let tx = p.db.unchecked_transaction()?;
    tx.execute("DELETE FROM detections WHERE model = ?1", [MODEL_ID])?;
    {
        let mut ins = tx.prepare("INSERT INTO detections (file_id, model, start_s, end_s, species, confidence, in_range) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1) RETURNING id")?;
        let mut rev = tx.prepare("INSERT INTO reviews (detection_id, verdict, at) VALUES (?1, ?2, datetime('now'))")?;
        for &(f, s, e, sp, c) in &found {
            let id: i64 = ins.query_row(params![f, MODEL_ID, s, e, sp, c as f64], |r| r.get(0))?;
            if let Some(v) = kept.get(&(f, (s * 10.0) as i64, sp)) { rev.execute(params![id, v])?; }
        }
    }
    tx.commit()?;
    Ok(found.len())
}

/// Calls `f(row, embedding)` for every embedding, reading the file in large chunks so memory stays small.
fn stream(p: &Project, mut f: impl FnMut(i64, &[f32])) -> Result<()> {
    let mut path = p.path.clone().into_os_string();
    path.push(".emb");
    let Ok(file) = std::fs::File::open(path) else { return Ok(()) };
    let mut r = std::io::BufReader::with_capacity(1 << 22, file);
    let mut buf = vec![0u8; EMBED * 2];
    let mut x = vec![0f32; EMBED];
    let mut row = 0i64;
    while r.read_exact(&mut buf).is_ok() {
        for (v, c) in x.iter_mut().zip(buf.chunks_exact(2)) { *v = half::f16::from_le_bytes([c[0], c[1]]).to_f32(); }
        f(row, &x);
        row += 1;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct Similar { pub window_id: i64, pub file_id: i64, pub path: String, pub start_s: f64, pub end_s: f64, pub similarity: f32, pub site: String }

/// The windows that sound most like a given one (cosine similarity of Perch embeddings), for "find more like this".
pub fn similar(p: &Project, window_id: i64, n: usize) -> Result<Vec<Similar>> {
    let row: i64 = p.db.query_row("SELECT row FROM windows WHERE id = ?1", [window_id], |r| r.get(0))?;
    let q = p.read_embeddings(&[row], EMBED)?.remove(0);
    let qn = dot(&q, &q).sqrt().max(1e-6);
    let mut best: Vec<(f32, i64)> = Vec::new();
    stream(p, |r, x| {
        if r == row { return; }
        let s = dot(&q, x) / (qn * dot(x, x).sqrt().max(1e-6));
        if best.len() < n || s > best.last().unwrap().0 {
            let at = best.partition_point(|b| b.0 > s);
            best.insert(at, (s, r));
            best.truncate(n);
        }
    })?;
    let mut out = Vec::new();
    for (s, r) in best {
        let w = p.db.query_row("SELECT w.id, w.file_id, f.path, w.start_s, w.end_s, d.name FROM windows w JOIN files f ON f.id = w.file_id
            JOIN deployments d ON d.id = f.deployment_id WHERE w.row = ?1 AND w.model = 'perch-2.0'", [r],
            |x| Ok(Similar { window_id: x.get(0)?, file_id: x.get(1)?, path: x.get(2)?, start_s: x.get(3)?, end_s: x.get(4)?, similarity: s, site: x.get(5)? }));
        if let Ok(w) = w { out.push(w); }
    }
    Ok(out)
}

/// The Perch window a detection sits in (the one overlapping it most), for naming it or finding more like it.
pub fn window_for(p: &Project, detection: i64) -> Result<Option<i64>> {
    let (file, s, e): (i64, f64, f64) = p.db.query_row("SELECT file_id, start_s, end_s FROM detections WHERE id = ?1", [detection], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let mid = (s + e) / 2.0;
    Ok(p.db.query_row("SELECT id FROM windows WHERE file_id = ?1 AND model = 'perch-2.0' AND start_s <= ?2 AND end_s > ?2", params![file, mid], |r| r.get(0)).ok())
}

/// "This window is (or is not) species X": an example for the next training. X may be a species no model knows.
pub fn add_example(p: &Project, window_id: i64, scientific: &str, common: &str, present: bool) -> Result<()> {
    if scientific.trim().len() < 3 { bail!("Please give the species' scientific name."); }
    let sp = p.species_id(scientific, common)?;
    p.db.execute("INSERT INTO examples (window_id, species, present, at) VALUES (?1, ?2, ?3, datetime('now'))
        ON CONFLICT (window_id, species) DO UPDATE SET present = excluded.present, at = excluded.at", params![window_id, sp, present as i64])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn learns_a_separable_class() {
        // Two clusters in 4 dimensions: the classifier must separate them and cross-validate well.
        let mk = |c: f32, i: usize| vec![c + (i % 3) as f32 * 0.01, -c, 0.1 * (i % 2) as f32, 1.0];
        let data: Vec<Vec<f32>> = (0..20).map(|i| mk(1.0, i)).chain((0..40).map(|i| mk(-1.0, i))).collect();
        let ys: Vec<f32> = (0..60).map(|i| if i < 20 { 1.0 } else { 0.0 }).collect();
        let xs: Vec<&Vec<f32>> = data.iter().collect();
        let (w, b) = fit(&xs, &ys);
        assert!(sigmoid(dot(&w, &mk(1.0, 5)) + b) > 0.9);
        assert!(sigmoid(dot(&w, &mk(-1.0, 5)) + b) < 0.1);
        let mut idx: Vec<usize> = (0..60).collect();
        idx.sort_by_key(|&i| (i * 2654435761) % 1000003);
        let (xs2, ys2): (Vec<&Vec<f32>>, Vec<f32>) = idx.iter().map(|&i| (xs[i], ys[i])).unzip();
        assert!(cv(&xs2, &ys2).unwrap() > 0.95);
    }
}
