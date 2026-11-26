//! Video frames through the camera-trap models: FFmpeg hands out frames at a fixed rate (1 a second unless the project
//! says otherwise), each goes through SpeciesNet (MegaDetector v6 finds the animals, people and vehicles, SpeciesNet names
//! them), and every box is kept with its time and how sharp it is. sightings.rs then joins the boxes of one animal across
//! frames.
use crate::camtrap::{self, Found, Rgb};
use crate::models::Classifier;
use crate::store::Project;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::params;
use serde::Serialize;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const SCHEMA: &str = r#"
-- One row per video and image model: 'done' or 'error', and the frames per second it was read at.
CREATE TABLE IF NOT EXISTS video_models (video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE, model TEXT NOT NULL,
  status TEXT NOT NULL, error TEXT, frames INTEGER, per_s REAL, processed_at TEXT NOT NULL, PRIMARY KEY (video_id, model));
-- Every box the models found in a sampled frame: t = seconds from the start of the video, box as fractions of the frame,
-- the same fields as image_boxes, and sharp = how sharp the box is (variance of the Laplacian of its grey levels).
CREATE TABLE IF NOT EXISTS video_boxes (
  id INTEGER PRIMARY KEY, video_id INTEGER NOT NULL REFERENCES videos(id) ON DELETE CASCADE, model TEXT NOT NULL, t REAL NOT NULL,
  category TEXT NOT NULL, conf REAL NOT NULL, x REAL NOT NULL, y REAL NOT NULL, w REAL NOT NULL, h REAL NOT NULL,
  kind TEXT NOT NULL, label TEXT, score REAL NOT NULL, source TEXT, top TEXT, sharp REAL NOT NULL);
CREATE INDEX IF NOT EXISTS video_boxes_video ON video_boxes (video_id, model, t);
"#;

/// Frames read per second of video unless the project sets "video_frames_per_s".
pub const DEFAULT_PER_S: f64 = 1.0;

pub fn per_s(p: &Project) -> f64 {
    p.setting("video_frames_per_s").ok().flatten().and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(DEFAULT_PER_S).clamp(0.1, 10.0)
}

/// Calls `on_frame(t, frame)` for frames `per_s` times a second (t in seconds from the start), until it returns false.
/// FFmpeg turns the frames upright (phone videos) and writes them as PPM, so each carries its own size.
pub fn read_frames(path: &Path, per_s: f64, mut on_frame: impl FnMut(f64, Rgb) -> Result<bool>) -> Result<usize> {
    let mut child = crate::video::command()?
        .args(["-v", "error", "-i"]).arg(path)
        .args(["-an", "-vf"]).arg(format!("fps={per_s}"))
        .args(["-f", "image2pipe", "-c:v", "ppm", "-"])
        .stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().with_context(|| format!("could not run FFmpeg on {}", path.display()))?;
    let mut err = child.stderr.take().unwrap();
    let errors = std::thread::spawn(move || { let mut s = String::new(); let _ = err.read_to_string(&mut s); s });
    let mut out = BufReader::with_capacity(1 << 20, child.stdout.take().unwrap());
    let mut n = 0usize;
    let mut stopped = false;
    let result = (|| -> Result<()> {
        while let Some(im) = read_ppm(&mut out)? {
            if !on_frame(n as f64 / per_s, im)? { stopped = true; break; }
            n += 1;
        }
        Ok(())
    })();
    let _ = child.kill();
    let status = child.wait()?;
    let errors = errors.join().unwrap_or_default();
    result?;
    if n == 0 && !stopped && !status.success() { bail!("FFmpeg could not read the frames of {}: {}", path.display(), errors.lines().last().unwrap_or("").trim()); }
    Ok(n)
}

/// One binary PPM (P6, 8 bit) from the stream; None at the end.
fn read_ppm(r: &mut impl BufRead) -> Result<Option<Rgb>> {
    let mut fields: Vec<usize> = Vec::with_capacity(3);
    let mut magic = [0u8; 2];
    match r.read_exact(&mut magic) { Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None), Err(e) => return Err(e.into()) }
    if &magic != b"P6" { bail!("unexpected frame data from FFmpeg"); }
    let mut tok = String::new();
    while fields.len() < 3 {
        let mut b = [0u8; 1];
        r.read_exact(&mut b)?;
        let c = b[0] as char;
        if c == '#' { let mut skip = Vec::new(); r.read_until(b'\n', &mut skip)?; continue; }
        if c.is_ascii_whitespace() { if !tok.is_empty() { fields.push(tok.parse()?); tok.clear(); } } else { tok.push(c); }
    }
    let (w, h, max) = (fields[0], fields[1], fields[2]);
    if max != 255 || w == 0 || h == 0 || w * h > 100_000_000 { bail!("unexpected frame format from FFmpeg ({w}x{h}, {max})"); }
    let mut px = vec![0u8; w * h * 3];
    r.read_exact(&mut px)?;
    Ok(Some(Rgb { w, h, px }))
}

/// How sharp the part of a frame inside a box is: the variance of the 4-neighbour Laplacian of its grey levels, at most
/// 256 pixels across (bigger boxes are sampled), so a blurred or moving animal scores low.
pub fn sharpness(im: &Rgb, b: [f64; 4]) -> f64 {
    let x0 = ((b[0].clamp(0.0, 1.0)) * im.w as f64) as usize;
    let y0 = ((b[1].clamp(0.0, 1.0)) * im.h as f64) as usize;
    let x1 = (((b[0] + b[2]).clamp(0.0, 1.0)) * im.w as f64) as usize;
    let y1 = (((b[1] + b[3]).clamp(0.0, 1.0)) * im.h as f64) as usize;
    if x1 < x0 + 3 || y1 < y0 + 3 { return 0.0; }
    let step = ((x1 - x0).max(y1 - y0) / 256).max(1);
    let grey = |x: usize, y: usize| { let i = (y * im.w + x) * 3; 0.299 * im.px[i] as f64 + 0.587 * im.px[i + 1] as f64 + 0.114 * im.px[i + 2] as f64 };
    let (mut n, mut sum, mut sq) = (0f64, 0f64, 0f64);
    let mut y = y0 + step;
    while y + step < y1 {
        let mut x = x0 + step;
        while x + step < x1 {
            let l = grey(x - step, y) + grey(x + step, y) + grey(x, y - step) + grey(x, y + step) - 4.0 * grey(x, y);
            n += 1.0; sum += l; sq += l * l;
            x += step;
        }
        y += step;
    }
    if n < 1.0 { 0.0 } else { (sq / n - (sum / n).powi(2)).max(0.0) }
}

/// Videos FFmpeg has read that `model` has not yet run on.
pub fn todo(p: &Project, model: &str) -> Result<i64> {
    Ok(p.db.query_row("SELECT COUNT(*) FROM videos v WHERE v.probed = 1 AND v.width IS NOT NULL AND NOT EXISTS (SELECT 1 FROM video_models m WHERE m.video_id = v.id AND m.model = ?1)", [model], |r| r.get(0))?)
}

pub fn total(p: &Project) -> Result<i64> { Ok(p.db.query_row("SELECT COUNT(*) FROM videos WHERE probed = 1 AND width IS NOT NULL", [], |r| r.get(0))?) }

#[derive(Debug, Default, Clone, Serialize)]
pub struct RunStats { pub videos: i64, pub failed: i64, pub frames: i64, pub boxes: i64, pub sightings: i64, pub seconds: f64 }

/// Runs every video not yet processed through each image model. `on_video(done, total, path)` before each video.
/// Each video is committed on its own (boxes and sightings together), so stopping loses at most the video in hand.
pub fn run(p: &Project, models: &[Arc<dyn Classifier>], stop: &AtomicBool, mut on_video: impl FnMut(i64, i64, &str)) -> Result<RunStats> {
    let mut stats = RunStats::default();
    if crate::video::ffmpeg().is_none() { return Ok(stats); }
    let started = std::time::Instant::now();
    let per_s = per_s(p);
    for m in crate::images::image_models(models) {
        let net = m.images().unwrap();
        let id = m.info().id.clone();
        let rows: Vec<(i64, String, i64)> = p.db.prepare("SELECT v.id, v.path, v.deployment_id FROM videos v WHERE v.probed = 1 AND v.width IS NOT NULL
            AND NOT EXISTS (SELECT 1 FROM video_models m WHERE m.video_id = v.id AND m.model = ?1) ORDER BY v.deployment_id, v.taken_local, v.path")?
            .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        let total = rows.len() as i64;
        for (k, (video_id, path, dep)) in rows.iter().enumerate() {
            if stop.load(Ordering::Relaxed) { break; }
            on_video(k as i64, total, path);
            let (country, admin1) = p.db.query_row("SELECT country, admin1 FROM deployments WHERE id = ?1", [dep], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))).unwrap_or((None, None));
            let mut boxes: Vec<(f64, Found, f64)> = Vec::new();
            let res = read_frames(Path::new(path), per_s, |t, im| {
                if stop.load(Ordering::Relaxed) { return Ok(false); }
                for f in net.process(&im, country.as_deref(), admin1.as_deref())? { let s = sharpness(&im, f.det.bbox); boxes.push((t, f, s)); }
                Ok(true)
            });
            if stop.load(Ordering::Relaxed) { break; }
            match res {
                Ok(frames) => {
                    let tx = p.db.unchecked_transaction()?;
                    tx.execute("DELETE FROM video_boxes WHERE video_id = ?1 AND model = ?2", params![video_id, id])?;
                    {
                        let mut ins = tx.prepare("INSERT INTO video_boxes (video_id, model, t, category, conf, x, y, w, h, kind, label, score, source, top, sharp)
                            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)")?;
                        for (t, f, sharp) in &boxes {
                            let b = f.det.bbox;
                            let (kind, label, score, source) = match &f.processed {
                                Some(pr) => (camtrap::taxon(&pr.label).kind.to_string(), Some(pr.label.clone()), pr.score, Some(pr.source.clone())),
                                // Boxes below SpeciesNet's classifying threshold keep the detector's own class.
                                None => (camtrap::CLASSES[f.det.class].to_string(), None, f.det.conf as f64, None),
                            };
                            let top: Vec<(String, f64)> = f.top.iter().map(|(l, s)| { let t = camtrap::taxon(l); (if t.common.is_empty() { t.scientific } else { t.common }, (*s * 10000.0).round() / 10000.0) }).collect();
                            ins.execute(params![video_id, id, t, camtrap::CLASSES[f.det.class], f.det.conf as f64, b[0], b[1], b[2], b[3], kind, label, score, source, serde_json::to_string(&top)?, sharp])?;
                        }
                    }
                    tx.execute("INSERT OR REPLACE INTO video_models (video_id, model, status, error, frames, per_s, processed_at) VALUES (?1, ?2, 'done', NULL, ?3, ?4, datetime('now'))",
                        params![video_id, id, frames as i64, per_s])?;
                    stats.sightings += crate::sightings::build(p, &tx, *video_id, &id)?;
                    tx.commit()?;
                    stats.videos += 1;
                    stats.frames += frames as i64;
                    stats.boxes += boxes.len() as i64;
                }
                Err(e) => {
                    p.db.execute("INSERT OR REPLACE INTO video_models (video_id, model, status, error, processed_at) VALUES (?1, ?2, 'error', ?3, datetime('now'))",
                        params![video_id, id, format!("{e:#}")])?;
                    stats.failed += 1;
                }
            }
        }
        on_video(total, total, "");
    }
    stats.seconds = started.elapsed().as_secs_f64();
    Ok(stats)
}

/// Sets the frames read per second; when it changes every video runs again on the next run. Returns whether it changed.
pub fn set_per_s(p: &Project, v: f64) -> Result<bool> {
    if !(0.1..=10.0).contains(&v) { bail!("Choose between 0.1 and 10 frames a second."); }
    if (per_s(p) - v).abs() < 1e-9 { return Ok(false); }
    p.set_setting("video_frames_per_s", &v.to_string())?;
    redo(p, None)?;
    Ok(true)
}

/// Runs a video again on the next run (after its camera's country changed, or the frame rate).
pub fn redo(p: &Project, video_id: Option<i64>) -> Result<usize> {
    Ok(match video_id {
        Some(v) => p.db.execute("DELETE FROM video_models WHERE video_id = ?1", [v])?,
        None => p.db.execute("DELETE FROM video_models", [])?,
    })
}

/// Reads one frame at `t` seconds as a JPEG (the still kept for a sighting).
pub fn still(path: &Path, t: f64, out: &Path) -> Result<()> {
    let st = crate::video::command()?.args(["-v", "error", "-y", "-ss"]).arg(format!("{t:.3}")).arg("-i").arg(path)
        .args(["-frames:v", "1", "-q:v", "2"]).arg(out).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
    if !st.success() || !out.exists() { return Err(anyhow!("could not take the frame at {t:.1} s from {}", path.display())); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_frames_at_the_chosen_rate() {
        let dir = tempfile::tempdir().unwrap();
        let Some(v) = crate::video::tests::make_video(dir.path(), "f.avi", 3, false) else { eprintln!("FFmpeg not installed: skipped"); return };
        let mut got = vec![];
        let n = read_frames(&v, 2.0, |t, im| { got.push((t, im.w, im.h)); Ok(true) }).unwrap();
        assert_eq!(n, 6);
        assert_eq!(got[0], (0.0, 320, 240));
        assert_eq!(got[5].0, 2.5);
        // Stopping early.
        let n = read_frames(&v, 1.0, |_, _| Ok(false)).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn sharp_beats_blurred() {
        let (w, h) = (64, 64);
        let mut sharp = Rgb { w, h, px: vec![0; w * h * 3] };
        for y in 0..h { for x in 0..w { let v = if (x / 4 + y / 4) % 2 == 0 { 255 } else { 0 }; for c in 0..3 { sharp.px[(y * w + x) * 3 + c] = v; } } }
        let mut soft = sharp.clone();
        for y in 0..h { for x in 0..w { for c in 0..3 { soft.px[(y * w + x) * 3 + c] = ((x * 4) % 256) as u8; } } }
        let b = [0.0, 0.0, 1.0, 1.0];
        assert!(sharpness(&sharp, b) > 10.0 * sharpness(&soft, b));
        assert_eq!(sharpness(&sharp, [0.5, 0.5, 0.0, 0.0]), 0.0);
    }
}
