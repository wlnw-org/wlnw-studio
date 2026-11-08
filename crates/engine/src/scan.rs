//! Import: find every recording under a folder or drive, read what each knows about itself, and group them into deployments
//! (one recorder at one place), by serial number when the files carry one, else by folder. Camera-trap images (JPEG, PNG)
//! found on the way are added too, grouped by folder the same way.
use crate::meta;
use crate::store::Project;
use anyhow::Result;
use rusqlite::params;
use serde::Serialize;
use std::path::Path;

const AUDIO: &[&str] = &["wav", "flac", "mp3"];

#[derive(Debug, Default, Serialize)]
pub struct ScanReport {
    pub found: usize, pub added: usize, pub without_time: usize, pub deployments: usize,
    /// Camera-trap images found, added, and added without a date (EXIF DateTimeOriginal).
    #[serde(default)] pub images_found: usize,
    #[serde(default)] pub images_added: usize,
    #[serde(default)] pub images_without_time: usize,
}

pub fn scan(project: &Project, root: &Path) -> Result<ScanReport> {
    let mut rep = ScanReport::default();
    // Full paths, so the project still finds its recordings whatever folder the app or CLI runs from.
    let root = &std::path::absolute(root)?;
    let tx = project.db.unchecked_transaction()?;
    for entry in walkdir::WalkDir::new(root).follow_links(true).into_iter().filter_map(|e| e.ok()) {
        let p = entry.path();
        if !entry.file_type().is_file() || p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')) { continue; }
        if crate::photo::is_image(p) { add_image(&tx, p, &entry, &mut rep)?; continue; }
        if !p.extension().and_then(|e| e.to_str()).is_some_and(|e| AUDIO.iter().any(|a| e.eq_ignore_ascii_case(a))) { continue; }
        rep.found += 1;
        let path = p.to_string_lossy().to_string();
        let exists: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM files WHERE path = ?1)", [&path], |r| r.get(0))?;
        if exists { continue; }
        let m = meta::read(p);
        let folder = p.parent().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let serial = m.serial.clone().unwrap_or_default();
        let name = if serial.is_empty() { p.parent().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Recorder".into()) } else { serial.clone() };
        // Recorders that note their time zone set it; for the others the user confirms it on the deployment.
        tx.execute(
            "INSERT INTO deployments (name, folder, serial, model, lat, lon, tz_offset_min, tz_known) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (folder, serial) DO UPDATE SET lat = COALESCE(deployments.lat, excluded.lat), lon = COALESCE(deployments.lon, excluded.lon),
               model = COALESCE(deployments.model, excluded.model),
               tz_offset_min = CASE WHEN deployments.tz_known = 1 THEN deployments.tz_offset_min ELSE excluded.tz_offset_min END,
               tz_known = MAX(deployments.tz_known, excluded.tz_known)",
            params![name, if serial.is_empty() { folder.clone() } else { String::new() }, serial, m.model, m.lat, m.lon, m.tz_offset_min.unwrap_or(0), m.tz_offset_min.is_some() as i64],
        )?;
        let dep: i64 = tx.query_row("SELECT id FROM deployments WHERE folder = ?1 AND serial = ?2",
            params![if serial.is_empty() { folder.clone() } else { String::new() }, serial], |r| r.get(0))?;
        if m.start_local.is_none() { rep.without_time += 1; }
        let bytes = entry.metadata().map(|md| md.len() as i64).unwrap_or(0);
        // The rate it was recorded at (header rate x time expansion), kept so the default model needs no header reads later.
        let rate = crate::audio::probe_rate(p).map(|r| (r as f64 * m.time_expansion.unwrap_or(1.0)).round() as i64);
        tx.execute("INSERT INTO files (deployment_id, path, start_local, time_source, bytes, sample_rate) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![dep, path, m.start_local.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()), m.time_source, bytes, rate])?;
        rep.added += 1;
    }
    tx.commit()?;
    rep.deployments = project.db.query_row("SELECT COUNT(*) FROM deployments", [], |r| r.get::<_, i64>(0))? as usize;
    Ok(rep)
}

/// One image: its deployment is its folder (as for recordings without a serial number); EXIF gives the time, the place
/// (filled into the deployment when it has none) and the camera model.
fn add_image(tx: &rusqlite::Transaction, p: &Path, entry: &walkdir::DirEntry, rep: &mut ScanReport) -> Result<()> {
    rep.images_found += 1;
    let path = p.to_string_lossy().to_string();
    let exists: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM images WHERE path = ?1)", [&path], |r| r.get(0))?;
    if exists { return Ok(()); }
    let m = crate::photo::read(p);
    let folder = p.parent().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let name = p.parent().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Camera".into());
    let model = match (&m.make, &m.model) { (Some(a), Some(b)) if !b.starts_with(a.as_str()) => Some(format!("{a} {b}")), (_, Some(b)) => Some(b.clone()), (Some(a), None) => Some(a.clone()), _ => None };
    tx.execute(
        "INSERT INTO deployments (name, folder, serial, model, lat, lon, tz_offset_min, tz_known) VALUES (?1, ?2, '', ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (folder, serial) DO UPDATE SET lat = COALESCE(deployments.lat, excluded.lat), lon = COALESCE(deployments.lon, excluded.lon),
           model = COALESCE(deployments.model, excluded.model),
           tz_offset_min = CASE WHEN deployments.tz_known = 1 THEN deployments.tz_offset_min ELSE excluded.tz_offset_min END,
           tz_known = MAX(deployments.tz_known, excluded.tz_known)",
        params![name, folder, model, m.lat, m.lon, m.tz_offset_min.unwrap_or(0), m.tz_offset_min.is_some() as i64],
    )?;
    let dep: i64 = tx.query_row("SELECT id FROM deployments WHERE folder = ?1 AND serial = ''", [&folder], |r| r.get(0))?;
    if m.taken_local.is_none() { rep.images_without_time += 1; }
    crate::images::add(tx, dep, &path, &m, entry.metadata().map(|md| md.len() as i64).unwrap_or(0))?;
    rep.images_added += 1;
    Ok(())
}

/// One line for the project history.
pub fn summary(r: &ScanReport, folder: &str) -> String {
    let mut s = format!("Added {} new of {} recordings found in {folder}", crate::history::n(r.added as i64), crate::history::n(r.found as i64));
    if r.images_found > 0 { s.push_str(&format!(", and {} new of {} camera-trap images", crate::history::n(r.images_added as i64), crate::history::n(r.images_found as i64))); }
    s.push_str(&format!(" ({} {})", r.deployments, if r.deployments == 1 { "recorder or camera" } else { "recorders and cameras" }));
    s
}
