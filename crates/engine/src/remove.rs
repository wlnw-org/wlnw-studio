//! Taking recorders, live stations or single recordings out of a project. Only Studio's own records go: the recordings'
//! rows, their detections, review answers (this project's and a second reviewer's), Perch windows and named examples,
//! camera-trap images of a removed recorder, and the clips Studio itself fetched from a live station. The audio files on
//! disk are never touched. Embeddings already written to `<project>.emb` stay in that file (new ones are appended after
//! them), they are simply no longer used.
use crate::live;
use crate::review;
use crate::store::Project;
use anyhow::{bail, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What to remove. Recorders (deployments) take all their recordings and images with them; a live station takes every
/// deployment it filled (one per place it reported) and its saved settings.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Target {
    #[serde(default)] pub deployments: Vec<i64>,
    #[serde(default)] pub files: Vec<i64>,
    /// Live station keys ("base:<id>", "bng:<host:port>").
    #[serde(default)] pub stations: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Counts {
    pub recorders: i64,
    pub recordings: i64,
    pub images: i64,
    pub detections: i64,
    /// Review answers (yes, no, not sure) on those detections.
    pub reviews: i64,
    /// Clips Studio fetched from live stations and kept next to the project.
    pub clips: i64,
}

/// Fills temporary tables rm_deps and rm_files with what `t` covers.
fn select(db: &Connection, t: &Target) -> Result<()> {
    db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS rm_deps (id INTEGER PRIMARY KEY); CREATE TEMP TABLE IF NOT EXISTS rm_files (id INTEGER PRIMARY KEY);
        DELETE FROM rm_deps; DELETE FROM rm_files;")?;
    {
        let mut st = db.prepare("INSERT OR IGNORE INTO rm_deps SELECT id FROM deployments WHERE id = ?1")?;
        for d in &t.deployments { st.execute([d])?; }
        // A station's deployments: "live:<key>", and "live:<key>@<day>" for each place a moved Base reported.
        let mut st = db.prepare("INSERT OR IGNORE INTO rm_deps SELECT id FROM deployments WHERE serial = '' AND (folder = ?1 OR substr(folder, 1, length(?1) + 1) = ?1 || '@')")?;
        for k in &t.stations { st.execute([format!("{}{k}", live::PREFIX)])?; }
    }
    db.execute_batch("INSERT OR IGNORE INTO rm_files SELECT f.id FROM files f JOIN rm_deps d ON d.id = f.deployment_id;")?;
    let mut st = db.prepare("INSERT OR IGNORE INTO rm_files SELECT id FROM files WHERE id = ?1")?;
    for f in &t.files { st.execute([f])?; }
    Ok(())
}

fn count(db: &Connection) -> Result<Counts> {
    let n = |sql: &str| -> Result<i64> { Ok(db.query_row(sql, [], |r| r.get(0))?) };
    Ok(Counts {
        recorders: n("SELECT COUNT(*) FROM rm_deps")?,
        recordings: n("SELECT COUNT(*) FROM rm_files")?,
        images: n("SELECT COUNT(*) FROM images WHERE deployment_id IN (SELECT id FROM rm_deps)")?,
        detections: n("SELECT COUNT(*) FROM detections WHERE file_id IN (SELECT id FROM rm_files)")?,
        reviews: n("SELECT COUNT(*) FROM reviews r JOIN detections d ON d.id = r.detection_id WHERE d.file_id IN (SELECT id FROM rm_files)")?,
        clips: n("SELECT COUNT(*) FROM live_clips WHERE file_id IN (SELECT id FROM rm_files)")?,
    })
}

fn clear(db: &Connection) { let _ = db.execute_batch("DROP TABLE IF EXISTS temp.rm_deps; DROP TABLE IF EXISTS temp.rm_files;"); }

/// What removing `t` would take out, for the confirmation.
pub fn preview(p: &Project, t: &Target) -> Result<Counts> {
    select(&p.db, t)?;
    let c = count(&p.db);
    clear(&p.db);
    c
}

/// The folder Studio keeps live station clips in ("<project> clips"): only files inside it are ever deleted.
fn clips_root(project: &Path) -> PathBuf { live::clip_dir(project, "x").parent().map(Path::to_path_buf).unwrap_or_default() }

/// Removes `t` from the project (see the module notes) and returns what went. Thresholds fitted from removed answers
/// are fitted again from the answers left.
pub fn remove(p: &Project, t: &Target) -> Result<Counts> {
    if t.deployments.is_empty() && t.files.is_empty() && t.stations.is_empty() { bail!("Nothing chosen to remove."); }
    let tx = p.db.unchecked_transaction()?;
    select(&tx, t)?;
    let counts = count(&tx)?;
    let clips: Vec<String> = tx.prepare("SELECT f.path FROM live_clips l JOIN files f ON f.id = l.file_id WHERE l.file_id IN (SELECT id FROM rm_files)")?
        .query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let refit: Vec<(i64, String)> = tx.prepare("SELECT DISTINCT d.species, d.model FROM detections d JOIN reviews r ON r.detection_id = d.id WHERE d.file_id IN (SELECT id FROM rm_files)")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    // Children first, each in one pass (the cascades would look rows up one by one, and reviews_other has no index on
    // detection_id alone). "shared" remembers ids sent to wlnw.org; a later detection could reuse a removed id.
    tx.execute_batch("
        CREATE TEMP TABLE IF NOT EXISTS rm_dets (id INTEGER PRIMARY KEY); DELETE FROM rm_dets;
        INSERT INTO rm_dets SELECT id FROM detections WHERE file_id IN (SELECT id FROM rm_files);
        CREATE TEMP TABLE IF NOT EXISTS rm_wins (id INTEGER PRIMARY KEY); DELETE FROM rm_wins;
        INSERT INTO rm_wins SELECT id FROM windows WHERE file_id IN (SELECT id FROM rm_files);
        DELETE FROM shared WHERE (kind = 'review' AND ref IN (SELECT id FROM rm_dets)) OR (kind = 'example' AND ref IN (SELECT id FROM rm_wins));
        DELETE FROM reviews_other WHERE detection_id IN (SELECT id FROM rm_dets);
        DELETE FROM reviews WHERE detection_id IN (SELECT id FROM rm_dets);
        DELETE FROM detections WHERE id IN (SELECT id FROM rm_dets);
        DELETE FROM examples WHERE window_id IN (SELECT id FROM rm_wins);
        DELETE FROM windows WHERE id IN (SELECT id FROM rm_wins);
        DELETE FROM file_models WHERE file_id IN (SELECT id FROM rm_files);
        DELETE FROM live_clips WHERE file_id IN (SELECT id FROM rm_files);
        DELETE FROM files WHERE id IN (SELECT id FROM rm_files);
        DELETE FROM images WHERE deployment_id IN (SELECT id FROM rm_deps);
        DELETE FROM deployments WHERE id IN (SELECT id FROM rm_deps);
        DROP TABLE temp.rm_dets; DROP TABLE temp.rm_wins;")?;
    for k in &t.stations { tx.execute("DELETE FROM settings WHERE key = ?1", [format!("{}{k}", live::PREFIX)])?; }
    clear(&tx);
    tx.commit()?;
    for (species, model) in refit { review::fit_threshold(p, species, &model, review::TARGET)?; }
    // Clips Studio downloaded (never the user's recordings): only inside "<project> clips".
    let root = clips_root(&p.path);
    for c in clips {
        let f = Path::new(&c);
        if f.starts_with(&root) { let _ = std::fs::remove_file(f); }
    }
    for k in &t.stations {
        let d = live::clip_dir(&p.path, k);
        if d.starts_with(&root) && d != root { let _ = std::fs::remove_dir_all(&d); }
    }
    let _ = std::fs::remove_dir(&root); // only when empty
    Ok(counts)
}

/// One line for the History screen.
pub fn summary(c: &Counts) -> String {
    let mut parts = vec![format!("{} recordings", crate::history::n(c.recordings))];
    if c.images > 0 { parts.push(format!("{} images", crate::history::n(c.images))); }
    parts.push(format!("{} detections", crate::history::n(c.detections)));
    if c.reviews > 0 { parts.push(format!("{} review answers", crate::history::n(c.reviews))); }
    let what = if c.recorders > 0 { format!("{} recorders or stations with ", crate::history::n(c.recorders)) } else { String::new() };
    format!("Removed {what}{} from the project. The audio files were left on disk.", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(&dir.path().join("t.wlnw")).unwrap();
        // Two recorders with two recordings each; detections, reviews, a second reviewer's answer, Perch windows and an example.
        p.db.execute_batch("
            INSERT INTO deployments (id, name, folder, serial) VALUES (1, 'A', '/a', 'A'), (2, 'B', '/b', 'B');
            INSERT INTO files (id, deployment_id, path, start_local) VALUES (1, 1, '/a/1.wav', '2025-11-20 05:00:00'), (2, 1, '/a/2.wav', '2025-11-20 06:00:00'),
              (3, 2, '/b/1.wav', '2025-11-20 05:00:00'), (4, 2, '/b/2.wav', '2025-11-20 06:00:00');
            INSERT INTO species (id, scientific, common) VALUES (1, 'Turdus merula', 'Eurasian Blackbird');
            INSERT INTO file_models (file_id, model, status, processed_at) VALUES (1, 'perch-2.0', 'done', 'now'), (2, 'perch-2.0', 'done', 'now'), (3, 'perch-2.0', 'done', 'now'), (4, 'perch-2.0', 'done', 'now');
            INSERT INTO windows (id, file_id, model, start_s, end_s, row) VALUES (1, 1, 'perch-2.0', 0, 5, 0), (2, 3, 'perch-2.0', 0, 5, 1);
            INSERT INTO examples (window_id, species, present, at) VALUES (1, 1, 1, 'now'), (2, 1, 1, 'now');
            INSERT INTO shared (kind, ref, at) VALUES ('example', 1, 'now'), ('example', 2, 'now');").unwrap();
        let mut id = 0;
        for file in 1..=4 { for k in 0..10 {
            id += 1;
            p.db.execute("INSERT INTO detections (id, file_id, model, start_s, end_s, species, confidence, in_range) VALUES (?1, ?2, 'perch-2.0', ?3, ?3 + 5, 1, ?4, 1)",
                params![id, file, k as f64 * 5.0, 0.15 + 0.08 * k as f64]).unwrap();
            if k % 2 == 0 { p.db.execute("INSERT INTO reviews VALUES (?1, ?2, 'now')", params![id, if file <= 2 { "no" } else { "yes" }]).unwrap(); }
        } }
        p.db.execute_batch("INSERT INTO reviews_other (reviewer, detection_id, verdict, at) VALUES ('Ann', 1, 'yes', 'now'), ('Ann', 21, 'yes', 'now');
            INSERT INTO shared (kind, ref, at) VALUES ('review', 1, 'now'), ('review', 21, 'now');").unwrap();
        (dir, p)
    }
    fn n(p: &Project, sql: &str) -> i64 { p.db.query_row(sql, [], |r| r.get(0)).unwrap() }

    #[test]
    fn a_recorder_goes_with_everything_studio_kept_for_it() {
        let (_d, p) = project();
        review::fit_threshold(&p, 1, "perch-2.0", review::TARGET).unwrap();
        let before = p.threshold(1, "perch-2.0");
        let t = Target { deployments: vec![1], ..Default::default() };
        let c = preview(&p, &t).unwrap();
        assert_eq!(c, Counts { recorders: 1, recordings: 2, images: 0, detections: 20, reviews: 10, clips: 0 });
        // A preview changes nothing.
        assert_eq!(n(&p, "SELECT COUNT(*) FROM detections"), 40);
        assert_eq!(remove(&p, &t).unwrap(), c);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM deployments"), 1);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM files WHERE deployment_id = 1"), 0);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM files"), 2);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM detections"), 20);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM detections d JOIN files f ON f.id = d.file_id WHERE f.deployment_id = 2"), 20);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM reviews"), 10);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM reviews_other"), 1);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM windows"), 1);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM examples"), 1);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM file_models"), 2);
        let shared: String = p.db.query_row("SELECT group_concat(kind || ref, ',') FROM (SELECT * FROM shared ORDER BY kind, ref)", [], |r| r.get(0)).unwrap();
        assert_eq!(shared, "example2,review21");
        // The threshold is fitted again from the answers left: recorder A's "no" answers no longer count.
        assert!(before > 1.0, "before {before}");
        assert!((p.threshold(1, "perch-2.0") - 0.1).abs() < 1e-9, "after {}", p.threshold(1, "perch-2.0"));
    }

    #[test]
    fn single_recordings_leave_the_rest_of_their_recorder() {
        let (_d, p) = project();
        let t = Target { files: vec![2, 3], ..Default::default() };
        assert_eq!(remove(&p, &t).unwrap(), Counts { recorders: 0, recordings: 2, images: 0, detections: 20, reviews: 10, clips: 0 });
        assert_eq!(n(&p, "SELECT COUNT(*) FROM deployments"), 2);
        let ids: String = p.db.query_row("SELECT group_concat(id, ',') FROM (SELECT id FROM files ORDER BY id)", [], |r| r.get(0)).unwrap();
        assert_eq!(ids, "1,4");
        assert_eq!(n(&p, "SELECT COUNT(*) FROM detections WHERE file_id IN (1, 4)"), 20);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM detections"), 20);
        // Windows and examples of recording 1 and 3: recording 3's went, recording 1's stayed.
        assert_eq!(n(&p, "SELECT COUNT(*) FROM examples"), 1);
        assert_eq!(n(&p, "SELECT window_id FROM examples"), 1);
    }

    #[test]
    fn a_recorder_with_camera_trap_images() {
        let (_d, p) = project();
        p.db.execute_batch("INSERT INTO images (id, deployment_id, path) VALUES (1, 2, '/b/i1.jpg'), (2, 2, '/b/i2.jpg'), (3, 1, '/a/i1.jpg');").unwrap();
        let c = remove(&p, &Target { deployments: vec![2], ..Default::default() }).unwrap();
        assert_eq!((c.recordings, c.images, c.detections), (2, 2, 20));
        assert_eq!(n(&p, "SELECT COUNT(*) FROM images"), 1);
        assert!(remove(&p, &Target::default()).is_err());
    }

    #[test]
    fn a_live_station_goes_with_its_places_settings_and_clips() {
        let (dir, p) = project();
        let key = "base:abc";
        let st = live::Station { key: key.into(), kind: "base".into(), name: "Hill".into(), address: "http://h".into(), api: "http://h:8080".into(),
            lat: Some(1.0), lon: Some(2.0), last: None, imported_at: None, folder: None, moved: None };
        p.set_setting(&format!("{}{key}", live::PREFIX), &serde_json::to_string(&st).unwrap()).unwrap();
        p.set_setting("live:bng:other:8080", "{}").unwrap();
        let cd = live::clip_dir(&p.path, key);
        std::fs::create_dir_all(&cd).unwrap();
        // Its first place and the place it moved to, plus another station that must stay.
        p.db.execute_batch("INSERT INTO deployments (id, name, folder, serial) VALUES (10, 'Hill', 'live:base:abc', ''), (11, 'Hill (moved)', 'live:base:abc@2026-11-20', ''),
            (12, 'Other', 'live:base:abcd', '');").unwrap();
        let other = live::clip_dir(&p.path, "base:abcd");
        std::fs::create_dir_all(&other).unwrap();
        for (file, dep) in [(10, 10), (11, 11), (12, 12)] {
            let path = if dep == 12 { other.join("12.wav") } else { cd.join(format!("{file}.wav")) };
            std::fs::write(&path, b"RIFF").unwrap();
            p.db.execute("INSERT INTO files (id, deployment_id, path) VALUES (?1, ?2, ?3)", params![file, dep, path.to_string_lossy()]).unwrap();
            p.db.execute("INSERT INTO live_clips (file_id, station, remote_id, url) VALUES (?1, ?2, ?1, 'u')", params![file, if dep == 12 { "base:abcd" } else { key }]).unwrap();
            p.db.execute("INSERT INTO detections (file_id, model, start_s, end_s, species, confidence, in_range) VALUES (?1, 'live', 0, 3, 1, 0.9, 1)", [file]).unwrap();
        }
        // A recording the user added next to the project: never deleted.
        let mine = dir.path().join("mine.wav");
        std::fs::write(&mine, b"RIFF").unwrap();
        let t = Target { stations: vec![key.into()], ..Default::default() };
        assert_eq!(preview(&p, &t).unwrap(), Counts { recorders: 2, recordings: 2, images: 0, detections: 2, reviews: 0, clips: 2 });
        remove(&p, &t).unwrap();
        assert_eq!(n(&p, "SELECT COUNT(*) FROM deployments WHERE folder LIKE 'live:%'"), 1);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM files WHERE id >= 10"), 1);
        assert_eq!(n(&p, "SELECT COUNT(*) FROM detections"), 41);
        assert!(p.setting("live:base:abc").unwrap().is_none());
        assert!(p.setting("live:bng:other:8080").unwrap().is_some());
        assert!(live::stations(&p).unwrap().iter().all(|s| s.key != key));
        // Its cached clips are gone; the other station's clip and the user's own file stay.
        assert!(!cd.exists());
        assert!(other.join("12.wav").exists() && mine.exists());
        assert_eq!(n(&p, "SELECT COUNT(*) FROM files"), 5);
    }
}
