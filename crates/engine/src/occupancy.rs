//! Occupancy tables for detection/non-detection models (for example R `unmarked`, `spOccupancy`):
//! - `<name>.csv`: one row per species and site, one column per day or week: 1 = detected (an accepted detection, the same
//!   ones every export uses), 0 = recorded and analysed but not detected, NA = nothing recorded that day or week;
//! - `<name>-effort.csv`: one row per site, the same columns, minutes of analysed recording.
//! Days are the recorder's local calendar days; weeks start on Monday. A recording that runs past midnight counts towards
//! both days. Recordings without a known start time cannot be placed in a day and are left out (counted in the report).
use crate::store::Project;
use anyhow::{bail, Result};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Period { Day, Week }

#[derive(Debug, Serialize)]
pub struct OccupancyReport { pub species: usize, pub sites: usize, pub periods: usize, pub detections: usize, pub files_without_time: usize, pub effort_file: String }

fn key(t: NaiveDateTime, per: Period) -> NaiveDate {
    let d = t.date();
    match per { Period::Day => d, Period::Week => d - Duration::days(d.weekday().num_days_from_monday() as i64) }
}

fn q(s: &str) -> String { if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() } }

/// The effort file sits next to the occupancy file: `x.csv` -> `x-effort.csv`.
pub fn effort_path(out: &Path) -> PathBuf {
    let stem = out.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "occupancy".into());
    out.with_file_name(format!("{stem}-effort.csv"))
}

pub fn export(p: &Project, out: &Path, per: Period) -> Result<OccupancyReport> {
    // Analysed recordings only: a file no model has finished is not effort.
    let mut st = p.db.prepare("SELECT f.id, f.deployment_id, f.start_local, f.duration_s FROM files f
        WHERE f.duration_s IS NOT NULL AND EXISTS (SELECT 1 FROM file_models m WHERE m.file_id = f.id AND m.status = 'done')")?;
    let files: Vec<(i64, i64, Option<String>, f64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    let mut start: HashMap<i64, (i64, NaiveDateTime)> = HashMap::new();
    let mut effort: BTreeMap<(i64, NaiveDate), f64> = BTreeMap::new(); // seconds
    let mut no_time = 0;
    for (id, dep, s, dur) in files {
        let Some(t0) = s.as_deref().and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()) else { no_time += 1; continue };
        start.insert(id, (dep, t0));
        let (mut t, mut left) = (t0, dur.max(0.0));
        while left > 1e-6 {
            let midnight = (t.date() + Duration::days(1)).and_hms_opt(0, 0, 0).unwrap();
            let seg = left.min((midnight - t).num_milliseconds() as f64 / 1000.0).max(1e-3);
            *effort.entry((dep, key(t, per))).or_default() += seg;
            t += Duration::milliseconds((seg * 1000.0).round() as i64);
            left -= seg;
        }
    }
    if effort.is_empty() { bail!("No analysed recordings with a known start time yet, so there is nothing to put in days."); }

    let mut detected: BTreeSet<(i64, i64, NaiveDate)> = BTreeSet::new(); // species, site, period
    let mut names: BTreeMap<i64, (String, String)> = BTreeMap::new();
    let mut used = 0;
    for d in p.accepted()? {
        let Some(&(dep, t0)) = start.get(&d.file_id) else { continue };
        let t = t0 + Duration::milliseconds((d.start_s * 1000.0) as i64);
        detected.insert((d.label, dep, key(t, per)));
        names.entry(d.label).or_insert((d.scientific.clone(), d.common.clone()));
        used += 1;
    }

    let first = effort.keys().map(|k| k.1).min().unwrap();
    let last = effort.keys().map(|k| k.1).max().unwrap();
    let step = if per == Period::Day { 1 } else { 7 };
    let cols: Vec<NaiveDate> = std::iter::successors(Some(first), |d| Some(*d + Duration::days(step))).take_while(|d| *d <= last).collect();
    let sites: Vec<_> = p.deployments()?.into_iter().filter(|s| effort.keys().any(|k| k.0 == s.id)).collect();
    let coord = |v: Option<f64>| v.map_or(String::new(), |x| x.to_string());
    let header: String = cols.iter().map(|d| format!(",{}", d.format("%Y-%m-%d"))).collect();

    let mut f = std::io::BufWriter::new(std::fs::File::create(out)?);
    writeln!(f, "scientific_name,common_name,site,latitude,longitude{header}")?;
    // Species sorted by scientific name, so a table can be compared between exports.
    let mut species: Vec<_> = names.iter().collect();
    species.sort_by(|a, b| a.1.0.cmp(&b.1.0));
    for (sp, (sci, common)) in &species {
        for s in &sites {
            write!(f, "{},{},{},{},{}", q(sci), q(common), q(&s.name), coord(s.lat), coord(s.lon))?;
            for c in &cols {
                let v = if detected.contains(&(**sp, s.id, *c)) { "1" } else if effort.contains_key(&(s.id, *c)) { "0" } else { "NA" };
                write!(f, ",{v}")?;
            }
            writeln!(f)?;
        }
    }
    f.flush()?;

    let ep = effort_path(out);
    let mut e = std::io::BufWriter::new(std::fs::File::create(&ep)?);
    writeln!(e, "site,latitude,longitude{header}")?;
    for s in &sites {
        write!(e, "{},{},{}", q(&s.name), coord(s.lat), coord(s.lon))?;
        for c in &cols { write!(e, ",{:.1}", effort.get(&(s.id, *c)).copied().unwrap_or(0.0) / 60.0)?; }
        writeln!(e)?;
    }
    e.flush()?;
    Ok(OccupancyReport { species: species.len(), sites: sites.len(), periods: cols.len(), detections: used, files_without_time: no_time, effort_file: ep.to_string_lossy().to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn days_weeks_and_missing_effort() {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(&dir.path().join("t.wlnw")).unwrap();
        // Site A: one hour from 23:30 on 1 June (runs past midnight) and nothing on 2 June after that; 4 June 10 minutes.
        // Site B: 10 minutes on 1 June only. One file without a time.
        p.db.execute_batch("INSERT INTO deployments (id, name, folder, serial, lat, lon) VALUES (1, 'A', '/a', 'A', 51.5, -0.1), (2, 'B', '/b', 'B', 51.6, -0.2);
            INSERT INTO files (id, deployment_id, path, start_local, duration_s) VALUES
              (1, 1, '/a/1.wav', '2025-06-01 23:30:00', 3600), (2, 1, '/a/2.wav', '2025-06-04 06:00:00', 600),
              (3, 2, '/b/1.wav', '2025-06-01 06:00:00', 600), (4, 2, '/b/x.wav', NULL, 600), (5, 2, '/b/notrun.wav', '2025-06-09 06:00:00', 600);
            INSERT INTO file_models (file_id, model, status, processed_at) VALUES (1,'birdnet-2.4','done',''),(2,'birdnet-2.4','done',''),(3,'birdnet-2.4','done',''),(4,'birdnet-2.4','done','');
            INSERT INTO species (id, scientific, common) VALUES (1, 'Turdus merula', 'Eurasian Blackbird');
            -- 45 min into file 1 = 00:15 on 2 June.
            INSERT INTO detections (file_id, model, start_s, end_s, species, confidence, in_range) VALUES (1, 'birdnet-2.4', 2700, 2703, 1, 0.9, 1);").unwrap();
        let out = dir.path().join("occ.csv");
        let r = export(&p, &out, Period::Day).unwrap();
        assert_eq!((r.species, r.sites, r.periods, r.detections, r.files_without_time), (1, 2, 4, 1, 1));
        let occ = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<&str> = occ.lines().collect();
        assert_eq!(lines[0], "scientific_name,common_name,site,latitude,longitude,2025-06-01,2025-06-02,2025-06-03,2025-06-04");
        assert_eq!(lines[1], "Turdus merula,Eurasian Blackbird,A,51.5,-0.1,0,1,NA,0");
        assert_eq!(lines[2], "Turdus merula,Eurasian Blackbird,B,51.6,-0.2,0,NA,NA,NA");
        let eff = std::fs::read_to_string(dir.path().join("occ-effort.csv")).unwrap();
        assert_eq!(eff.lines().nth(1).unwrap(), "A,51.5,-0.1,30.0,30.0,0.0,10.0");

        let w = export(&p, &out, Period::Week).unwrap();
        assert_eq!(w.periods, 2); // Sunday 1 June 2025 ends the week of 26 May; 2 to 4 June are in the week of 2 June
        let occ = std::fs::read_to_string(&out).unwrap();
        assert!(occ.starts_with("scientific_name,common_name,site,latitude,longitude,2025-05-26,2025-06-02\n"));
        assert!(occ.contains("A,51.5,-0.1,0,1\n") && occ.contains("B,51.6,-0.2,0,NA\n"));
    }
}
