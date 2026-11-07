//! Every recording in a project, one row each, with where it stands for each model it should run through.
use crate::run::plan;
use crate::store::Project;
use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub model: String,
    /// done, error or todo.
    pub status: String,
    pub error: Option<String>,
    pub processed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileRow {
    pub id: i64, pub path: String, pub deployment_id: i64, pub recorder: String,
    pub start_local: Option<String>, pub bytes: i64, pub duration_s: Option<f64>,
    /// done (every model), partial (some), todo (none yet), failed (a model could not read it), none (no model for its recorder).
    pub status: &'static str,
    pub models: Vec<ModelStatus>,
    pub detections: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Counts { pub all: usize, pub done: usize, pub partial: usize, pub todo: usize, pub failed: usize, pub none: usize }

#[derive(Debug, Clone, Serialize)]
pub struct Page { pub rows: Vec<FileRow>, pub total: usize, pub counts: Counts }

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Query {
    /// One of the statuses, or empty for all.
    #[serde(default)] pub status: Option<String>,
    #[serde(default)] pub deployment: Option<i64>,
    /// Part of the file name or folder, any case.
    #[serde(default)] pub search: Option<String>,
    #[serde(default)] pub offset: usize,
    #[serde(default)] pub limit: usize,
}

fn rows(p: &Project, have: &[String], q: &Query) -> Result<(Vec<FileRow>, Counts)> {
    let plan = plan(p, have)?;
    let mut fm: HashMap<i64, Vec<(String, String, Option<String>, String)>> = HashMap::new();
    {
        let mut st = p.db.prepare("SELECT file_id, model, status, error, processed_at FROM file_models")?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))? { let (f, v) = r?; fm.entry(f).or_default().push(v); }
    }
    let search = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")));
    let mut st = p.db.prepare(
        "SELECT f.id, f.path, f.deployment_id, d.name, f.start_local, COALESCE(f.bytes, 0), f.duration_s
         FROM files f JOIN deployments d ON d.id = f.deployment_id
         WHERE (?1 IS NULL OR f.deployment_id = ?1) AND (?2 IS NULL OR f.path LIKE ?2 ESCAPE '\\')
         ORDER BY d.name, f.start_local, f.path")?;
    let mut counts = Counts::default();
    let mut out = Vec::new();
    for r in st.query_map(params![q.deployment, search], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?,
        r.get::<_, Option<String>>(4)?, r.get::<_, i64>(5)?, r.get::<_, Option<f64>>(6)?)))? {
        let (id, path, dep, recorder, start_local, bytes, duration_s) = r?;
        let have_rows = fm.remove(&id).unwrap_or_default();
        let wanted = plan.get(&dep).cloned().unwrap_or_default();
        let mut models: Vec<ModelStatus> = wanted.iter().map(|m| match have_rows.iter().find(|x| &x.0 == m) {
            Some((_, s, e, at)) => ModelStatus { model: m.clone(), status: s.clone(), error: e.clone(), processed_at: Some(at.clone()) },
            None => ModelStatus { model: m.clone(), status: "todo".into(), error: None, processed_at: None },
        }).collect();
        // Results from models no longer chosen still count as history for the file.
        for (m, s, e, at) in have_rows.iter().filter(|x| !wanted.contains(&x.0)) {
            models.push(ModelStatus { model: m.clone(), status: s.clone(), error: e.clone(), processed_at: Some(at.clone()) });
        }
        let planned = &models[..wanted.len()];
        let status = if wanted.is_empty() { "none" }
            else if planned.iter().any(|m| m.status == "error") { "failed" }
            else if planned.iter().all(|m| m.status == "done") { "done" }
            else if planned.iter().any(|m| m.status == "done") { "partial" }
            else { "todo" };
        counts.all += 1;
        match status { "done" => counts.done += 1, "partial" => counts.partial += 1, "failed" => counts.failed += 1, "none" => counts.none += 1, _ => counts.todo += 1 }
        if q.status.as_deref().is_some_and(|s| !s.is_empty() && s != status) { continue; }
        out.push(FileRow { id, path, deployment_id: dep, recorder, start_local, bytes, duration_s, status, models, detections: 0 });
    }
    Ok((out, counts))
}

/// One page of recordings (`limit` 0 = 200), with the counts per status for the same recorder and search.
pub fn list(p: &Project, have: &[String], q: &Query) -> Result<Page> {
    let (all, counts) = rows(p, have, q)?;
    let total = all.len();
    let limit = if q.limit == 0 { 200 } else { q.limit.min(2000) };
    let mut rows: Vec<FileRow> = all.into_iter().skip(q.offset).take(limit).collect();
    let mut st = p.db.prepare("SELECT COUNT(*) FROM detections WHERE file_id = ?1")?;
    for r in &mut rows { r.detections = st.query_row([r.id], |x| x.get(0))?; }
    Ok(Page { rows, total, counts })
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// Every recording matching the query as CSV: one row per recording, models and errors in their own columns.
pub fn csv(p: &Project, have: &[String], q: &Query, out: &Path) -> Result<usize> {
    let (rows, _) = rows(p, have, q)?;
    let mut counts: HashMap<i64, i64> = HashMap::new();
    {
        let mut st = p.db.prepare("SELECT file_id, COUNT(*) FROM detections GROUP BY file_id")?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? { let (f, c) = r?; counts.insert(f, c); }
    }
    let mut w = String::from("path,recorder,start_local,bytes,duration_s,status,detections,models,processed_at,errors\n");
    for r in &rows {
        let models = r.models.iter().map(|m| format!("{}={}", m.model, m.status)).collect::<Vec<_>>().join("; ");
        let last = r.models.iter().filter_map(|m| m.processed_at.clone()).max().unwrap_or_default();
        let errors = r.models.iter().filter_map(|m| m.error.as_ref().map(|e| format!("{}: {e}", m.model))).collect::<Vec<_>>().join("; ");
        w.push_str(&[csv_field(&r.path), csv_field(&r.recorder), r.start_local.clone().unwrap_or_default(), r.bytes.to_string(),
            r.duration_s.map(|d| format!("{d:.1}")).unwrap_or_default(), r.status.to_string(), counts.get(&r.id).copied().unwrap_or(0).to_string(),
            csv_field(&models), last, csv_field(&errors)].join(","));
        w.push('\n');
    }
    std::fs::write(out, w)?;
    Ok(rows.len())
}

/// Forgets failed attempts (one recording, or all) so the next run tries them again. Returns how many were cleared.
pub fn retry_failed(p: &Project, file: Option<i64>) -> Result<usize> {
    Ok(p.db.execute("DELETE FROM file_models WHERE status = 'error' AND (?1 IS NULL OR file_id = ?1)", [file])?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_retry() {
        let dir = std::env::temp_dir().join(format!("wlnw-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = Project::open(&dir.join("t.wlnw")).unwrap();
        p.set_setting("models", r#"["a","b"]"#).unwrap();
        p.db.execute_batch("INSERT INTO deployments (id, name, folder) VALUES (1, 'North', '/x');
            INSERT INTO files (id, deployment_id, path, bytes) VALUES (1, 1, '/x/one.wav', 10), (2, 1, '/x/two.wav', 10), (3, 1, '/x/three.wav', 10), (4, 1, '/x/Four, \"q\".wav', 10);
            INSERT INTO file_models VALUES (1, 'a', 'done', NULL, '2026-11-07 10:00:00'), (1, 'b', 'done', NULL, '2026-11-07 10:01:00'),
                (2, 'a', 'done', NULL, '2026-11-07 10:02:00'), (3, 'a', 'error', 'not audio', '2026-11-07 10:03:00');").unwrap();
        let have = vec!["a".to_string(), "b".to_string()];
        let page = list(&p, &have, &Query::default()).unwrap();
        let c = &page.counts;
        assert_eq!((c.all, c.done, c.partial, c.failed, c.todo), (4, 1, 1, 1, 1));
        let failed = list(&p, &have, &Query { status: Some("failed".into()), ..Default::default() }).unwrap();
        assert_eq!(failed.rows.len(), 1);
        assert_eq!(failed.rows[0].models[0].error.as_deref(), Some("not audio"));
        let s = list(&p, &have, &Query { search: Some("TWO".into()), ..Default::default() }).unwrap();
        assert_eq!(s.total, 1);
        // Only model "a" installed: file 2 is done, file 1 keeps b's result as history.
        let only_a = list(&p, &["a".to_string()], &Query::default()).unwrap();
        assert_eq!(only_a.counts.done, 2);
        let out = dir.join("files.csv");
        assert_eq!(csv(&p, &have, &Query::default(), &out).unwrap(), 4);
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.contains("\"/x/Four, \"\"q\"\".wav\""));
        assert!(text.contains("a: not audio"));
        assert_eq!(retry_failed(&p, None).unwrap(), 1);
        assert_eq!(list(&p, &have, &Query::default()).unwrap().counts.failed, 0);
        drop(p);
        std::fs::remove_dir_all(&dir).ok();
    }
}
