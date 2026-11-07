//! Processing: every recording, with every model chosen for its recorder, one recording and model at a time, committed
//! when finished. Safe to stop and start again: a (recording, model) pair already done is skipped.
use crate::audio;
use crate::birdnet::RANGE_THRESHOLD;
use crate::meta::birdnet_week;
use crate::models::{default_models, Classifier};
use crate::history;
use crate::store::Project;
use anyhow::Result;
use chrono::NaiveDateTime;
use rusqlite::params;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub const BATCH: usize = 16;

#[derive(Debug, Clone, Serialize, Default)]
pub struct Progress {
    /// Recording and model pairs done / to do.
    pub jobs_done: i64, pub jobs_total: i64,
    pub files_done: i64, pub files_total: i64, pub hours_done: f64, pub bytes_done: i64, pub bytes_total: i64,
    pub detections: i64, pub speed_x: f64, pub eta_s: Option<f64>, pub current: Option<String>, pub current_model: Option<String>, pub errors: i64,
}

/// Model ids to run for each deployment: the project's choice, or the regional default.
pub fn plan(p: &Project, have: &[String]) -> Result<HashMap<i64, Vec<String>>> {
    let chosen: Vec<String> = p.chosen_models().into_iter().filter(|m| have.contains(m)).collect();
    Ok(p.deployments()?.into_iter().map(|d| (d.id, if chosen.is_empty() { default_models(d.lat, d.lon, have) } else { chosen.clone() })).collect())
}

pub fn progress(p: &Project, plan: &HashMap<i64, Vec<String>>) -> Result<Progress> {
    let mut pr = Progress::default();
    let mut st = p.db.prepare("SELECT f.id, f.deployment_id, COALESCE(f.bytes, 0), f.duration_s FROM files f")?;
    let files: Vec<(i64, i64, i64, Option<f64>)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    let mut done: HashMap<(i64, String), String> = HashMap::new();
    let mut st = p.db.prepare("SELECT file_id, model, status FROM file_models")?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? { let (f, m, s) = r?; done.insert((f, m), s); }
    for (id, dep, bytes, dur) in files {
        let models = plan.get(&dep).cloned().unwrap_or_default();
        let n_done = models.iter().filter(|m| done.contains_key(&(id, (*m).clone()))).count() as i64;
        pr.errors += models.iter().filter(|m| done.get(&(id, (*m).clone())).is_some_and(|s| s == "error")).count() as i64;
        pr.jobs_total += models.len() as i64;
        pr.jobs_done += n_done;
        pr.files_total += 1;
        pr.bytes_total += bytes * models.len() as i64;
        pr.bytes_done += bytes * n_done;
        if !models.is_empty() && n_done == models.len() as i64 { pr.files_done += 1; pr.hours_done += dur.unwrap_or(0.0) / 3600.0; }
    }
    pr.detections = p.db.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))?;
    Ok(pr)
}

/// Perch keeps one embedding per 5 s window next to the project (1,536 half floats = 3,072 bytes), so about 614 bytes per
/// second of audio. Below this much free space on the project's disk, a run stops before the next file.
pub const EMB_BYTES_PER_SECOND: f64 = 3072.0 / 5.0;
pub const MIN_FREE_BYTES: u64 = 1 << 30;

/// Embedding bytes per byte of a recording not processed yet (its length is unknown until it is read): typical
/// recorder formats, 48 kHz 16-bit mono WAV (96 KB/s), FLAC about half that, MP3 about 128 kbit/s.
fn emb_per_audio_byte(path: &str) -> f64 {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let audio_bytes_per_s = match ext.as_str() { "flac" => 48_000.0, "mp3" => 16_000.0, _ => 96_000.0 };
    EMB_BYTES_PER_SECOND / audio_bytes_per_s
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Space {
    /// Disk space the embeddings of the recordings still to run through an embedding model will take (an estimate).
    pub needed: u64,
    /// Free space on the disk that holds the project, when it can be read.
    pub free: Option<u64>,
    /// False when the estimate plus a 1 GB margin is more than the free space.
    pub enough: bool,
}

pub fn space(p: &Project, plan: &HashMap<i64, Vec<String>>, embedding_models: &[String]) -> Result<Space> {
    let mut needed = 0f64;
    if !embedding_models.is_empty() {
        let mut done: std::collections::HashSet<(i64, String)> = std::collections::HashSet::new();
        let mut st = p.db.prepare("SELECT file_id, model FROM file_models")?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? { done.insert(r?); }
        let mut st = p.db.prepare("SELECT id, deployment_id, path, COALESCE(bytes, 0), duration_s FROM files")?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, Option<f64>>(4)?)))? {
            let (id, dep, path, bytes, dur) = r?;
            for m in plan.get(&dep).into_iter().flatten().filter(|m| embedding_models.contains(m)) {
                if done.contains(&(id, m.clone())) { continue; }
                needed += match dur { Some(d) => d * EMB_BYTES_PER_SECOND, None => bytes as f64 * emb_per_audio_byte(&path) };
            }
        }
    }
    let needed = needed.round() as u64;
    let free = fs4::available_space(p.path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."))).ok();
    Ok(Space { needed, free, enough: free.is_none_or(|f| f >= needed + MIN_FREE_BYTES) })
}

/// What one run did, for the project history.
#[derive(Default)]
struct RunStats { files: HashMap<i64, f64>, failed: std::collections::HashSet<i64> }

/// Runs every (recording, model) pair still to do. `on_progress` is called before each pair; set `stop` to finish the current pair and return.
/// Each run is written to the project history with what it did.
pub fn run(p: &Project, models: &[Arc<dyn Classifier>], stop: &AtomicBool, on_progress: impl FnMut(&Progress)) -> Result<Progress> {
    history::close_interrupted(p)?;
    let have: Vec<String> = models.iter().map(|m| m.info().id.clone()).collect();
    let mut used: Vec<String> = plan(p, &have)?.into_values().flatten().collect();
    used.sort();
    used.dedup();
    let names = used.iter().map(|id| models.iter().find(|m| &m.info().id == id).map(|m| m.info().name.clone()).unwrap_or_else(|| id.clone())).collect::<Vec<_>>().join(" + ");
    let names = if names.is_empty() { "No model".to_string() } else { names };
    let count = || -> i64 { p.db.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0)).unwrap_or(0) };
    let before = count();
    let hid = history::begin(p, "run", &format!("{names}: running"), &serde_json::json!({ "models": used }))?;
    let started = Instant::now();
    let mut st = RunStats::default();
    let r = run_inner(p, models, stop, on_progress, &mut st);
    let audio: f64 = st.files.values().sum::<f64>().max(0.0);
    let added = count() - before;
    let wall = started.elapsed().as_secs_f64();
    let mut parts = if st.files.is_empty() { vec!["no recording read".to_string()] } else {
        vec![format!("{} {} ({} of audio)", history::n(st.files.len() as i64), if st.files.len() == 1 { "recording" } else { "recordings" }, history::hours(audio)),
            format!("{} new detections", history::n(added))]
    };
    if !st.failed.is_empty() { parts.push(format!("{} could not be read", history::n(st.failed.len() as i64))); }
    let (status, tail) = match &r {
        Ok(pr) if pr.jobs_done >= pr.jobs_total => ("finished", if st.failed.is_empty() { "Every recording is done." } else { "Every recording has been tried." }.to_string()),
        Ok(pr) => ("paused", format!("Paused with {} left.", history::n(pr.files_total - pr.files_done))),
        Err(e) => ("failed", format!("Stopped by an error: {e:#}")),
    };
    let summary = format!("{names}: {}. {tail}", parts.join(", "));
    history::end(p, hid, status, &summary, &serde_json::json!({
        "models": used, "recordings": st.files.len(), "failed": st.failed.len(), "audio_s": audio, "detections": added,
        "seconds": wall, "speed_x": if wall > 0.0 { audio / wall } else { 0.0 },
    }))?;
    r
}

fn run_inner(p: &Project, models: &[Arc<dyn Classifier>], stop: &AtomicBool, mut on_progress: impl FnMut(&Progress), stats: &mut RunStats) -> Result<Progress> {
    let have: Vec<String> = models.iter().map(|m| m.info().id.clone()).collect();
    let plan = plan(p, &have)?;
    let mut species: HashMap<String, Vec<i64>> = HashMap::new();
    for m in models { species.insert(m.info().id.clone(), p.species_ids(m.labels())?); }
    let todo: Vec<(i64, String, Option<String>, i64, i64, Option<f64>, Option<f64>)> = {
        let mut st = p.db.prepare("SELECT f.id, f.path, f.start_local, COALESCE(f.bytes, 0), f.deployment_id, d.lat, d.lon FROM files f
            JOIN deployments d ON d.id = f.deployment_id ORDER BY f.deployment_id, f.start_local, f.path")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let started = Instant::now();
    let (mut audio_s, mut bytes_run) = (0f64, 0i64);
    let mut ranges: HashMap<(String, i64, i64, i32), Vec<bool>> = HashMap::new();
    'outer: for (id, path, start_local, bytes, dep, lat, lon) in todo {
        for model_id in plan.get(&dep).cloned().unwrap_or_default() {
            if stop.load(Ordering::Relaxed) { break 'outer; }
            let done: bool = p.db.query_row("SELECT EXISTS (SELECT 1 FROM file_models WHERE file_id = ?1 AND model = ?2)", params![id, model_id], |r| r.get(0))?;
            if done { continue; }
            let Some(model) = models.iter().find(|m| m.info().id == model_id) else { continue };
            if model.info().embeds {
                let dir = p.path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
                if fs4::available_space(dir).is_ok_and(|f| f < MIN_FREE_BYTES) {
                    anyhow::bail!("Less than 1 GB is free on the disk that holds the project, so processing stopped before the next file. Free some space or move the project to a bigger disk, then resume.");
                }
            }
            let mut prog = progress(p, &plan)?;
            let wall = started.elapsed().as_secs_f64();
            if wall > 1.0 && audio_s > 0.0 { prog.speed_x = audio_s / wall; prog.eta_s = Some((prog.bytes_total - prog.bytes_done) as f64 * wall / bytes_run.max(1) as f64); }
            prog.current = Some(path.clone());
            prog.current_model = Some(model.info().name.clone());
            on_progress(&prog);
            // Species expected at this place and week, from the model's own location model when it has one.
            let start = start_local.as_deref().and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok());
            let in_range: Option<Vec<bool>> = match (lat, lon, start, model.info().location_filter) {
                (Some(la), Some(lo), Some(t), true) => {
                    let key = (model_id.clone(), (la * 10.0).round() as i64, (lo * 10.0).round() as i64, birdnet_week(t));
                    if !ranges.contains_key(&key) {
                        if let Some(e) = model.expected(la, lo, key.3)? { ranges.insert(key.clone(), e.iter().map(|&v| v >= RANGE_THRESHOLD).collect()); }
                    }
                    ranges.get(&key).cloned()
                }
                _ => None,
            };
            match process_file(Path::new(&path), model.as_ref()) {
                Ok(out) => {
                    let rows = if out.embeddings.is_empty() { vec![] } else { p.append_embeddings(&out.embeddings)? };
                    let ids = &species[&model_id];
                    let tx = p.db.unchecked_transaction()?;
                    tx.execute("DELETE FROM detections WHERE file_id = ?1 AND model = ?2", params![id, model_id])?;
                    tx.execute("DELETE FROM windows WHERE file_id = ?1 AND model = ?2", params![id, model_id])?;
                    {
                        let mut ins = tx.prepare("INSERT INTO detections (file_id, model, start_s, end_s, species, confidence, in_range) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")?;
                        for (s, e, label, conf) in out.detections {
                            let ok = in_range.as_ref().map_or(true, |r| r.get(label).copied().unwrap_or(false));
                            ins.execute(params![id, model_id, s, e, ids[label], conf as f64, ok as i64])?;
                        }
                        let mut win = tx.prepare("INSERT INTO windows (file_id, model, start_s, end_s, row) VALUES (?1, ?2, ?3, ?4, ?5)")?;
                        for ((s, e), row) in out.windows.iter().zip(&rows) { win.execute(params![id, model_id, s, e, row])?; }
                    }
                    tx.execute("UPDATE files SET duration_s = ?2 WHERE id = ?1", params![id, out.seconds])?;
                    tx.execute("INSERT OR REPLACE INTO file_models (file_id, model, status, error, processed_at) VALUES (?1, ?2, 'done', NULL, datetime('now'))", params![id, model_id])?;
                    tx.commit()?;
                    audio_s += out.seconds;
                    stats.files.insert(id, out.seconds);
                }
                Err(e) => {
                    p.db.execute("INSERT OR REPLACE INTO file_models (file_id, model, status, error, processed_at) VALUES (?1, ?2, 'error', ?3, datetime('now'))",
                        params![id, model_id, format!("{e:#}")])?;
                    stats.failed.insert(id);
                }
            }
            bytes_run += bytes;
        }
    }
    let mut prog = progress(p, &plan)?;
    let wall = started.elapsed().as_secs_f64().max(1e-3);
    prog.speed_x = audio_s / wall;
    prog.eta_s = Some(0.0);
    on_progress(&prog);
    Ok(prog)
}

pub struct FileResult {
    /// (start s, end s, label index, confidence) at or above the model's floor.
    pub detections: Vec<(f64, f64, usize, f32)>,
    /// Windows (start s, end s) and their embeddings, for models that embed.
    pub windows: Vec<(f64, f64)>,
    pub embeddings: Vec<Vec<f32>>,
    pub seconds: f64,
}

pub fn process_file(path: &Path, model: &dyn Classifier) -> Result<FileResult> {
    let info = model.info();
    let rate = info.sample_rate as f64;
    let mut s = audio::open(path, info.sample_rate)?;
    let mut out = FileResult { detections: vec![], windows: vec![], embeddings: vec![], seconds: 0.0 };
    let floor = model.floor();
    loop {
        let mut batch = Vec::with_capacity(BATCH);
        let mut starts = Vec::with_capacity(BATCH);
        while batch.len() < BATCH {
            match s.next_window(info.window, info.min_len)? { Some((st, c)) => { starts.push(st); batch.push(c); } None => break }
        }
        if batch.is_empty() { break; }
        for (b, scored) in model.predict(&batch)?.into_iter().enumerate() {
            let (st, en) = (starts[b] as f64 / rate, (starts[b] + info.window) as f64 / rate);
            for (k, &c) in scored.scores.iter().enumerate() { if c >= floor { out.detections.push((st, en, k, c)); } }
            if let Some(e) = scored.embedding { out.windows.push((st, en)); out.embeddings.push(e); }
        }
    }
    out.seconds = s.position as f64 / rate;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_estimate() {
        let dir = std::env::temp_dir().join(format!("wlnw-space-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = Project::open(&dir.join("t.wlnw")).unwrap();
        p.db.execute_batch("INSERT INTO deployments (id, name, folder) VALUES (1, 'A', '/x');
            INSERT INTO files (id, deployment_id, path, bytes, duration_s) VALUES
              (1, 1, '/x/a.wav', 96000000, NULL), (2, 1, '/x/b.flac', 48000000, NULL), (3, 1, '/x/c.wav', 1, 3600.0), (4, 1, '/x/d.wav', 96000000, NULL);
            INSERT INTO file_models VALUES (4, 'perch-2.0', 'done', NULL, '2026-11-07 10:00:00');").unwrap();
        let plan: HashMap<i64, Vec<String>> = [(1, vec!["perch-2.0".to_string(), "birdnet-2.4".to_string()])].into();
        let s = space(&p, &plan, &["perch-2.0".to_string()]).unwrap();
        // 1,000 s of WAV + 1,000 s of FLAC + one known hour; file 4 is done.
        assert_eq!(s.needed, ((1000.0 + 1000.0 + 3600.0) * EMB_BYTES_PER_SECOND).round() as u64);
        assert!(s.free.is_some());
        assert_eq!(space(&p, &plan, &[]).unwrap().needed, 0);
        drop(p);
        std::fs::remove_dir_all(&dir).ok();
    }
}
