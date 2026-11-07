//! What has been done in a project and when: every processing run, folder added, training, export and publish, with a
//! one-line summary. Kept in the project file, so the history travels with the project.
use crate::store::Project;
use anyhow::Result;
use rusqlite::params;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: i64,
    /// run, scan, train, export or publish.
    pub kind: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    /// running, finished, paused, interrupted or failed.
    pub status: String,
    pub summary: String,
    pub detail: serde_json::Value,
}

/// Starts an entry that will be finished later (a processing run).
pub fn begin(p: &Project, kind: &str, summary: &str, detail: &serde_json::Value) -> Result<i64> {
    p.db.execute("INSERT INTO history (kind, started_at, status, summary, detail) VALUES (?1, datetime('now'), 'running', ?2, ?3)",
        params![kind, summary, detail.to_string()])?;
    Ok(p.db.last_insert_rowid())
}

pub fn end(p: &Project, id: i64, status: &str, summary: &str, detail: &serde_json::Value) -> Result<()> {
    p.db.execute("UPDATE history SET finished_at = datetime('now'), status = ?2, summary = ?3, detail = ?4 WHERE id = ?1",
        params![id, status, summary, detail.to_string()])?;
    Ok(())
}

/// A run left 'running' by a crash or a closed laptop.
pub fn close_interrupted(p: &Project) -> Result<()> {
    p.db.execute("UPDATE history SET status = 'interrupted' WHERE status = 'running'", [])?;
    Ok(())
}

/// Records a finished action from its result. `ok` writes the summary of a success; a failure keeps the error.
pub fn log<T>(p: &Project, kind: &str, started_at: &str, r: &Result<T>, ok: impl FnOnce(&T) -> String, failed: &str) {
    let (status, summary) = match r { Ok(v) => ("finished", ok(v)), Err(e) => ("failed", format!("{failed}: {e:#}")) };
    // History is a convenience: never let it turn a finished action into an error.
    let _ = p.db.execute("INSERT INTO history (kind, started_at, finished_at, status, summary, detail) VALUES (?1, ?2, datetime('now'), ?3, ?4, '{}')",
        params![kind, started_at, status, summary]);
}

/// The current time as SQLite writes it, for `log`'s `started_at`.
pub fn now(p: &Project) -> String {
    p.db.query_row("SELECT datetime('now')", [], |r| r.get(0)).unwrap_or_default()
}

pub fn list(p: &Project, limit: i64) -> Result<Vec<Entry>> {
    let mut st = p.db.prepare("SELECT id, kind, started_at, finished_at, status, summary, detail FROM history ORDER BY id DESC LIMIT ?1")?;
    let rows = st.query_map([limit], |r| Ok(Entry {
        id: r.get(0)?, kind: r.get(1)?, started_at: r.get(2)?, finished_at: r.get(3)?, status: r.get(4)?, summary: r.get(5)?,
        detail: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or(serde_json::Value::Null),
    }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// 12345 -> "12,345".
pub fn n(v: impl Into<i64>) -> String {
    let v: i64 = v.into();
    let s = v.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { out.push(','); } out.push(c); }
    if v < 0 { format!("-{out}") } else { out }
}

pub fn hours(seconds: f64) -> String {
    let h = seconds / 3600.0;
    if h < 1.0 { format!("{} min", (h * 60.0).round()) } else if h < 10.0 { format!("{h:.1} h") } else { format!("{} h", n(h.round() as i64)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers() {
        assert_eq!(n(0), "0");
        assert_eq!(n(999), "999");
        assert_eq!(n(1000), "1,000");
        assert_eq!(n(1234567), "1,234,567");
        assert_eq!(n(-4200), "-4,200");
        assert_eq!(hours(1800.0), "30 min");
        assert_eq!(hours(5400.0), "1.5 h");
        assert_eq!(hours(36000.0 * 12.0), "120 h");
    }

    #[test]
    fn entries() {
        let dir = std::env::temp_dir().join(format!("wlnw-hist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = Project::open(&dir.join("t.wlnw")).unwrap();
        let id = begin(&p, "run", "started", &serde_json::json!({})).unwrap();
        close_interrupted(&p).unwrap();
        let t = now(&p);
        log(&p, "export", &t, &Ok::<usize, anyhow::Error>(5), |k| format!("{k} rows"), "Export failed");
        log(&p, "publish", &t, &Err::<usize, _>(anyhow::anyhow!("offline")), |_| String::new(), "Publish failed");
        let l = list(&p, 10).unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].summary, "Publish failed: offline");
        assert_eq!(l[1].summary, "5 rows");
        assert_eq!((l[2].id, l[2].status.as_str()), (id, "interrupted"));
        drop(p);
        std::fs::remove_dir_all(&dir).ok();
    }
}
