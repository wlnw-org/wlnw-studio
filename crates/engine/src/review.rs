//! Review built for minutes, not weeks: per species, a few clips from every confidence band; each verdict sharpens
//! that species' threshold, fitted so that what is kept is right at least `target` of the time.
use crate::audio;
use crate::store::{Detection, Project};
use crate::SR;
use anyhow::{bail, Result};
use realfft::RealFftPlanner;
use rusqlite::params;
use serde::Serialize;
use std::path::Path;

/// Confidence bands: [0.1, 0.2), ... [0.9, 1.0].
pub const BANDS: [f64; 10] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.01];
pub const MIN_REVIEWS: i64 = 8;
pub const TARGET: f64 = 0.9;

fn band(c: f64) -> usize { BANDS.iter().position(|&b| c < b).unwrap_or(BANDS.len()).saturating_sub(1).min(8) }

#[derive(Debug, Serialize)]
pub struct SpeciesRow {
    pub label: i64, pub model: String, pub scientific: String, pub common: String, pub detections: i64, pub kept: i64,
    pub reviewed: i64, pub yes: i64, pub no: i64, pub threshold: f64, pub threshold_source: String, pub precision: Option<f64>, pub max_conf: f64,
}

/// Every species found in range, per model (each model's confidence means something different), most detections first.
pub fn species(p: &Project) -> Result<Vec<SpeciesRow>> {
    let mut st = p.db.prepare(
        "SELECT d.species, d.model, s.scientific, s.common, COUNT(*), COUNT(r.verdict), COUNT(*) FILTER (WHERE r.verdict = 'yes'), COUNT(*) FILTER (WHERE r.verdict = 'no'),
                t.threshold, t.source, t.precision, MAX(d.confidence)
         FROM detections d JOIN species s ON s.id = d.species LEFT JOIN reviews r ON r.detection_id = d.id
         LEFT JOIN thresholds t ON t.species = d.species AND t.model = d.model
         WHERE d.in_range = 1 GROUP BY d.species, d.model ORDER BY COUNT(*) DESC")?;
    let rows: Vec<SpeciesRow> = st.query_map([], |r| Ok(SpeciesRow {
        label: r.get(0)?, model: r.get(1)?, scientific: r.get(2)?, common: r.get(3)?, detections: r.get(4)?, kept: 0, reviewed: r.get(5)?, yes: r.get(6)?, no: r.get(7)?,
        threshold: r.get::<_, Option<f64>>(8)?.unwrap_or(f64::NAN), threshold_source: r.get::<_, Option<String>>(9)?.unwrap_or_else(|| "default".into()),
        precision: r.get(10)?, max_conf: r.get(11)?,
    }))?.collect::<Result<_, _>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for mut row in rows {
        if row.threshold.is_nan() { row.threshold = p.default_threshold(&row.model); }
        row.kept = p.db.query_row("SELECT COUNT(*) FROM detections d LEFT JOIN reviews r ON r.detection_id = d.id
            WHERE d.species = ?1 AND d.model = ?2 AND d.in_range = 1 AND ((d.confidence >= ?3 AND COALESCE(r.verdict, '') <> 'no') OR r.verdict = 'yes')",
            params![row.label, row.model, row.threshold], |r| r.get(0))?;
        out.push(row);
    }
    Ok(out)
}

/// The next clips to review for a species and model: unreviewed, spread over the confidence bands, highest band first.
pub fn queue(p: &Project, label: i64, model: &str, n: usize) -> Result<Vec<Detection>> {
    let mut out = Vec::new();
    let per = n.div_ceil(9).max(1) as i64;
    for b in (0..9).rev() {
        // A stable spread through the recordings rather than the first ones found.
        let picked = p.detections(&format!("d.species = ?1 AND d.model = ?4 AND d.in_range = 1 AND r.verdict IS NULL AND d.confidence >= ?2 AND d.confidence < ?3
            AND d.id IN (SELECT id FROM detections WHERE species = ?1 AND model = ?4 AND confidence >= ?2 AND confidence < ?3 ORDER BY (id * 2654435761) % 1000003 LIMIT {per})"),
            &[&label, &BANDS[b], &BANDS[b + 1], &model], per)?;
        out.extend(picked);
        if out.len() >= n { break; }
    }
    out.truncate(n);
    Ok(out)
}

pub fn set_verdict(p: &Project, detection: i64, verdict: Option<&str>) -> Result<()> {
    match verdict {
        None => { p.db.execute("DELETE FROM reviews WHERE detection_id = ?1", [detection])?; }
        Some(v @ ("yes" | "no" | "unsure")) => {
            p.db.execute("INSERT INTO reviews (detection_id, verdict, at) VALUES (?1, ?2, datetime('now'))
                ON CONFLICT (detection_id) DO UPDATE SET verdict = excluded.verdict, at = excluded.at", params![detection, v])?;
        }
        Some(_) => bail!("verdict must be yes, no or unsure"),
    }
    let (species, model): (i64, String) = p.db.query_row("SELECT species, model FROM detections WHERE id = ?1", [detection], |r| Ok((r.get(0)?, r.get(1)?)))?;
    fit_threshold(p, species, &model, TARGET)?;
    Ok(())
}

/// Fits a species' threshold for one model from its reviews. Each band's precision (yes / (yes + no)) is weighted by how many
/// detections the band holds, since clips were sampled evenly across bands. The threshold is the lowest band start at which
/// the estimated precision of everything kept reaches `target`. Bands without reviews take the nearest reviewed band above.
pub fn fit_threshold(p: &Project, species: i64, model: &str, target: f64) -> Result<Option<f64>> {
    let mut count = [0f64; 9];
    let mut yes = [0f64; 9];
    let mut no = [0f64; 9];
    let mut st = p.db.prepare("SELECT d.confidence, r.verdict FROM detections d LEFT JOIN reviews r ON r.detection_id = d.id WHERE d.species = ?1 AND d.model = ?2 AND d.in_range = 1")?;
    let mut rows = st.query(params![species, model])?;
    while let Some(r) = rows.next()? {
        let (c, v): (f64, Option<String>) = (r.get(0)?, r.get(1)?);
        let b = band(c);
        count[b] += 1.0;
        match v.as_deref() { Some("yes") => yes[b] += 1.0, Some("no") => no[b] += 1.0, _ => {} }
    }
    let reviewed: f64 = yes.iter().sum::<f64>() + no.iter().sum::<f64>();
    if (reviewed as i64) < MIN_REVIEWS {
        p.db.execute("DELETE FROM thresholds WHERE species = ?1 AND model = ?2 AND source = 'fitted'", params![species, model])?;
        return Ok(None);
    }
    let mut prec = [None; 9];
    for b in 0..9 { if yes[b] + no[b] > 0.0 { prec[b] = Some(yes[b] / (yes[b] + no[b])); } }
    for b in (0..9).rev() { if prec[b].is_none() { prec[b] = prec[b + 1..].iter().flatten().next().copied(); } }
    for b in 0..9 { if prec[b].is_none() { prec[b] = prec[..b].iter().rev().flatten().next().copied(); } }
    let mut chosen = (1.01, None);
    for t in 0..9 {
        let (w, s) = (t..9).fold((0.0, 0.0), |(w, s), b| (w + count[b], s + count[b] * prec[b].unwrap_or(0.0)));
        if w > 0.0 && s / w >= target { chosen = (BANDS[t], Some(s / w)); break; }
    }
    p.db.execute("INSERT INTO thresholds (species, model, threshold, precision, reviewed, source) VALUES (?1, ?2, ?3, ?4, ?5, 'fitted')
        ON CONFLICT (species, model) DO UPDATE SET threshold = excluded.threshold, precision = excluded.precision, reviewed = excluded.reviewed, source = 'fitted'",
        params![species, model, chosen.0, chosen.1, reviewed as i64])?;
    Ok(Some(chosen.0))
}

#[derive(Debug, Serialize)]
pub struct Clip { pub wav: Vec<u8>, pub spec: Vec<u8>, pub width: usize, pub height: usize, pub offset_s: f64, pub window_s: f64, pub total_s: f64, pub max_khz: f64 }

/// A clip around a detection (1 s either side) as WAV, and its spectrogram (0 to 12 kHz) as grey levels, top row highest frequency.
pub fn clip(path: &Path, start_s: f64, end_s: f64) -> Result<Clip> {
    let from = (start_s - 1.0).max(0.0);
    let samples = audio::read_segment(path, from, (end_s - from) + 1.0)?;
    let (n, hop, max_hz) = (512usize, 160usize, 12_000f64);
    let bins = ((max_hz / SR as f64) * n as f64) as usize;
    let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
    let win: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()).collect();
    let frames = if samples.len() > n { (samples.len() - n) / hop + 1 } else { 0 };
    let mut db = vec![0f32; frames * bins];
    let (mut buf, mut out) = (vec![0f32; n], fft.make_output_vec());
    for f in 0..frames {
        for i in 0..n { buf[i] = samples[f * hop + i] * win[i]; }
        fft.process(&mut buf, &mut out)?;
        for k in 0..bins { db[f * bins + k] = 10.0 * (out[k].norm_sqr() + 1e-10).log10(); }
    }
    let mut sorted = db.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (lo, hi) = (sorted.get(sorted.len() / 5).copied().unwrap_or(-100.0), sorted.last().copied().unwrap_or(0.0));
    let mut spec = vec![0u8; frames * bins];
    for f in 0..frames { for k in 0..bins {
        let v = ((db[f * bins + k] - lo) / (hi - lo).max(1e-3)).clamp(0.0, 1.0);
        spec[(bins - 1 - k) * frames + f] = (v * 255.0) as u8;
    } }
    Ok(Clip { wav: audio::wav_bytes(&samples, SR), spec, width: frames, height: bins, offset_s: start_s - from, window_s: end_s - start_s, total_s: samples.len() as f64 / SR as f64, max_khz: max_hz / 1000.0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(&dir.path().join("t.wlnw")).unwrap();
        p.db.execute_batch("INSERT INTO deployments (id, name, folder, serial, lat, lon) VALUES (1, 'A', '', 'A', 51.5, -0.1);
            INSERT INTO files (id, deployment_id, path, start_local) VALUES (1, 1, '/x/A_20251120_050000.wav', '2025-11-20 05:00:00');
            INSERT INTO species (id, scientific, common) VALUES (1, 'Turdus merula', 'Eurasian Blackbird');").unwrap();
        (dir, p)
    }
    #[test]
    fn threshold_from_reviews() {
        let (_d, p) = project();
        // 10 detections per band from 0.1 to 1.0; reviews say low bands are wrong, bands from 0.6 up are right.
        let mut id = 0;
        for b in 0..9 { for k in 0..10 {
            id += 1;
            p.db.execute("INSERT INTO detections (id, file_id, model, start_s, end_s, species, confidence, in_range) VALUES (?1, 1, 'birdnet-2.4', ?2, ?2 + 3, 1, ?3, 1)", params![id, id as f64 * 3.0, BANDS[b] + 0.01 * k as f64]).unwrap();
            if k < 2 { p.db.execute("INSERT INTO reviews VALUES (?1, ?2, 'now')", params![id, if b >= 5 { "yes" } else { "no" }]).unwrap(); }
        } }
        let t = fit_threshold(&p, 1, "birdnet-2.4", 0.9).unwrap().unwrap();
        assert!((t - 0.6).abs() < 1e-9, "threshold {t}");
        // Accepted = confidence >= 0.6 and not rejected: 40 detections, of which none were reviewed "no".
        assert_eq!(p.accepted().unwrap().len(), 40);
        let s = species(&p).unwrap();
        assert_eq!((s[0].reviewed, s[0].kept, s[0].threshold_source.as_str()), (18, 40, "fitted"));
    }
    #[test]
    fn too_few_reviews_keep_default() {
        let (_d, p) = project();
        p.db.execute("INSERT INTO detections VALUES (1, 1, 'birdnet-2.4', 0, 3, 1, 0.9, 1)", []).unwrap();
        set_verdict(&p, 1, Some("yes")).unwrap();
        assert_eq!(p.threshold(1, "birdnet-2.4"), 0.5);
        let dir = tempfile::tempdir().unwrap();
        let n = crate::export::dwca(&p, &dir.path().join("a.zip")).unwrap();
        assert_eq!(n, 1);
        let mut z = zip::ZipArchive::new(std::fs::File::open(dir.path().join("a.zip")).unwrap()).unwrap();
        let mut occ = String::new();
        std::io::Read::read_to_string(&mut z.by_name("occurrence.txt").unwrap(), &mut occ).unwrap();
        assert!(occ.contains("2025-11-20T05:00:00Z\tTurdus merula") && occ.contains("verified by expert"), "{occ}");
        let mut meta = String::new();
        std::io::Read::read_to_string(&mut z.by_name("meta.xml").unwrap(), &mut meta).unwrap();
        assert!(meta.contains("http://purl.org/dc/terms/license"), "{meta}");
    }
}
