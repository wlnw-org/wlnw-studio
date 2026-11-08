//! Two reviewers, one project: each reviews on their own copy, one sends their answers as a small .wlnr file, the other
//! loads it next to their own (table reviews_other, never touching reviews) and sees, per species, how often they agree
//! and Cohen's kappa on yes/no. Each disagreement can be settled; settling writes the final answer to reviews and keeps
//! the first answer, so the agreement figures stay those of the two independent reviews.
//! A detection is identified across copies by recording (full path, else file name), start, model and species, because
//! detection ids differ between copies.
use crate::store::{Detection, Project};
use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Answer { pub path: String, pub start_s: f64, pub model: String, pub species: String, pub verdict: String, #[serde(default)] pub at: String }

#[derive(Debug, Serialize, Deserialize)]
pub struct AnswerFile {
    pub format: String, pub version: u32, #[serde(default)] pub reviewer: Option<String>, #[serde(default)] pub project: String,
    #[serde(default)] pub exported_at: String, pub answers: Vec<Answer>,
}

/// Writes every answer given in this project to a .wlnr file. Returns how many.
pub fn export(p: &Project, out: &Path, reviewer: Option<&str>) -> Result<usize> {
    let mut st = p.db.prepare("SELECT f.path, d.start_s, d.model, s.scientific, r.verdict, r.at FROM reviews r JOIN detections d ON d.id = r.detection_id
        JOIN files f ON f.id = d.file_id JOIN species s ON s.id = d.species ORDER BY f.path, d.start_s")?;
    let answers: Vec<Answer> = st.query_map([], |r| Ok(Answer { path: r.get(0)?, start_s: r.get(1)?, model: r.get(2)?, species: r.get(3)?, verdict: r.get(4)?, at: r.get(5)? }))?
        .collect::<Result<_, _>>()?;
    let reviewer = reviewer.map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    if let Some(r) = &reviewer { p.set_setting("reviewer", r)?; }
    let file = AnswerFile {
        format: "wlnr".into(), version: 1, reviewer, project: p.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        exported_at: crate::history::now(p), answers,
    };
    std::fs::write(out, serde_json::to_string_pretty(&file)?)?;
    Ok(file.answers.len())
}

#[derive(Debug, Serialize)]
pub struct LoadReport { pub reviewer: String, pub answers: usize, pub matched: usize, pub unmatched: usize }

/// Loads another reviewer's .wlnr file under a label (the reviewer's name in the file, else `label`, else the file name).
/// Loading the same reviewer again updates their answers and keeps what was settled.
pub fn load(p: &Project, file: &Path, label: Option<&str>) -> Result<LoadReport> {
    let a: AnswerFile = serde_json::from_str(&std::fs::read_to_string(file)?).map_err(|e| anyhow::anyhow!("Not a review answers file (.wlnr): {e}"))?;
    if a.format != "wlnr" { bail!("Not a review answers file (.wlnr)."); }
    let reviewer = label.map(str::trim).filter(|s| !s.is_empty()).map(String::from).or(a.reviewer.clone().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Second reviewer".into()));
    // Recordings by full path, and by file name when that is unique (the other copy may live in another folder).
    let mut by_path = HashMap::new();
    let mut by_name: HashMap<String, Vec<i64>> = HashMap::new();
    {
        let mut st = p.db.prepare("SELECT id, path FROM files")?;
        for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (id, path) = r?;
            by_name.entry(name(&path)).or_default().push(id);
            by_path.insert(path.replace('\\', "/"), id);
        }
    }
    let mut rep = LoadReport { reviewer: reviewer.clone(), answers: a.answers.len(), matched: 0, unmatched: 0 };
    let tx = p.db.unchecked_transaction()?;
    {
        let mut find = tx.prepare("SELECT d.id FROM detections d JOIN species s ON s.id = d.species WHERE d.file_id = ?1 AND d.model = ?2 AND s.scientific = ?3
            AND abs(d.start_s - ?4) < 0.05 ORDER BY abs(d.start_s - ?4) LIMIT 1")?;
        let mut put = tx.prepare("INSERT INTO reviews_other (reviewer, detection_id, verdict, at) VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT (reviewer, detection_id) DO UPDATE SET verdict = excluded.verdict, at = excluded.at")?;
        for x in &a.answers {
            if !["yes", "no", "unsure"].contains(&x.verdict.as_str()) { rep.unmatched += 1; continue; }
            let file = by_path.get(&x.path.replace('\\', "/")).copied().or_else(|| match by_name.get(&name(&x.path)).map(Vec::as_slice) { Some([id]) => Some(*id), _ => None });
            let det: Option<i64> = match file { Some(f) => find.query_row(params![f, x.model, x.species, x.start_s], |r| r.get(0)).optional()?, None => None };
            match det {
                Some(d) => { put.execute(params![reviewer, d, x.verdict, x.at])?; rep.matched += 1; }
                None => rep.unmatched += 1,
            }
        }
    }
    tx.commit()?;
    Ok(rep)
}

fn name(path: &str) -> String { path.replace('\\', "/").rsplit('/').next().unwrap_or("").to_lowercase() }

/// Cohen's kappa for two raters on yes/no: a both yes, b first yes and second no, c first no and second yes, d both no.
/// None when there is nothing to compare or chance agreement is already total (both said the same thing every time).
pub fn kappa(a: f64, b: f64, c: f64, d: f64) -> Option<f64> {
    let n = a + b + c + d;
    if n == 0.0 { return None; }
    let po = (a + d) / n;
    let pe = ((a + b) * (a + c) + (c + d) * (b + d)) / (n * n);
    if (1.0 - pe).abs() < 1e-12 { return None; }
    Some((po - pe) / (1.0 - pe))
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SpeciesAgreement {
    pub scientific: String, pub common: String,
    /// Detections both reviewed (any answer), and those where both said yes or no (unsure left out).
    pub both: i64, pub yes_no: i64,
    /// Both yes, first yes and second no, first no and second yes, both no.
    pub yy: i64, pub yn: i64, pub ny: i64, pub nn: i64,
    pub percent: Option<f64>, pub kappa: Option<f64>,
}
impl SpeciesAgreement {
    fn add(&mut self, mine: &str, other: &str) {
        self.both += 1;
        match (mine, other) { ("yes", "yes") => self.yy += 1, ("yes", "no") => self.yn += 1, ("no", "yes") => self.ny += 1, ("no", "no") => self.nn += 1, _ => return }
        self.yes_no += 1;
    }
    fn finish(&mut self) {
        let (a, b, c, d) = (self.yy as f64, self.yn as f64, self.ny as f64, self.nn as f64);
        self.percent = (self.yes_no > 0).then(|| 100.0 * (a + d) / self.yes_no as f64);
        self.kappa = kappa(a, b, c, d);
    }
}

#[derive(Debug, Serialize)]
pub struct Disagreement { #[serde(flatten)] pub detection: Detection, pub other: String }

#[derive(Debug, Serialize)]
pub struct Report {
    /// Reviewers loaded, newest first; `reviewer` is the one compared.
    pub reviewers: Vec<String>, pub reviewer: Option<String>,
    pub overall: SpeciesAgreement, pub species: Vec<SpeciesAgreement>,
    /// Still to settle (any differing answers, unsure included), and how many were settled already.
    pub disagreements: Vec<Disagreement>, pub settled: i64,
}

pub fn reviewers(p: &Project) -> Result<Vec<String>> {
    let mut st = p.db.prepare("SELECT reviewer FROM reviews_other GROUP BY reviewer ORDER BY MAX(rowid) DESC")?;
    let out = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(out)
}

/// Agreement between this project's own answers and one other reviewer's (the newest loaded when `reviewer` is None).
pub fn report(p: &Project, reviewer: Option<&str>) -> Result<Report> {
    let all = reviewers(p)?;
    let Some(who) = reviewer.map(String::from).filter(|r| all.contains(r)).or_else(|| all.first().cloned()) else {
        return Ok(Report { reviewers: all, reviewer: None, overall: SpeciesAgreement::default(), species: vec![], disagreements: vec![], settled: 0 });
    };
    // The first answer counts for agreement: settling changes reviews but leaves mine_before.
    let mut st = p.db.prepare("SELECT s.scientific, s.common, COALESCE(o.mine_before, r.verdict), o.verdict FROM reviews_other o
        JOIN reviews r ON r.detection_id = o.detection_id JOIN detections d ON d.id = o.detection_id JOIN species s ON s.id = d.species WHERE o.reviewer = ?1")?;
    let mut per: BTreeMap<String, SpeciesAgreement> = BTreeMap::new();
    let mut overall = SpeciesAgreement { scientific: "All species".into(), ..Default::default() };
    for r in st.query_map([&who], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))? {
        let (sci, common, mine, other) = r?;
        per.entry(sci.clone()).or_insert_with(|| SpeciesAgreement { scientific: sci, common, ..Default::default() }).add(&mine, &other);
        overall.add(&mine, &other);
    }
    overall.finish();
    let mut species: Vec<SpeciesAgreement> = per.into_values().map(|mut s| { s.finish(); s }).collect();
    species.sort_by(|a, b| b.both.cmp(&a.both).then(a.scientific.cmp(&b.scientific)));
    let mut other: HashMap<i64, String> = HashMap::new();
    {
        let mut st = p.db.prepare("SELECT o.detection_id, o.verdict FROM reviews_other o JOIN reviews r ON r.detection_id = o.detection_id
            WHERE o.reviewer = ?1 AND o.settled_at IS NULL AND r.verdict <> o.verdict LIMIT 2000")?;
        for r in st.query_map([&who], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? { let (d, v) = r?; other.insert(d, v); }
    }
    let ids = other.keys().map(i64::to_string).collect::<Vec<_>>().join(",");
    let disagreements = if ids.is_empty() { vec![] } else {
        p.detections(&format!("d.id IN ({ids})"), &[], 2000)?.into_iter().map(|d| Disagreement { other: other[&d.id].clone(), detection: d }).collect()
    };
    let settled = p.db.query_row("SELECT COUNT(*) FROM reviews_other WHERE reviewer = ?1 AND settled_at IS NOT NULL", [&who], |r| r.get(0))?;
    Ok(Report { reviewers: all, reviewer: Some(who), overall, species, disagreements, settled })
}

/// Settles a disagreement: `verdict` becomes the project's answer (and refits that species' threshold); the first answer is kept for the figures.
pub fn settle(p: &Project, reviewer: &str, detection: i64, verdict: &str) -> Result<()> {
    let mine: Option<String> = p.db.query_row("SELECT verdict FROM reviews WHERE detection_id = ?1", [detection], |r| r.get(0)).optional()?;
    let n = p.db.execute("UPDATE reviews_other SET mine_before = COALESCE(mine_before, ?3), settled_at = datetime('now') WHERE reviewer = ?1 AND detection_id = ?2",
        params![reviewer, detection, mine])?;
    if n == 0 { bail!("{reviewer} did not review that detection."); }
    crate::review::set_verdict(p, detection, Some(verdict))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kappa_math() {
        // Textbook case: 50 items, 20 both yes, 15 both no, 5 + 10 disagree. po 0.7, pe 0.5, kappa 0.4.
        assert!((kappa(20.0, 5.0, 10.0, 15.0).unwrap() - 0.4).abs() < 1e-12);
        // Perfect agreement with both categories used is 1; systematic disagreement is -1.
        assert_eq!(kappa(10.0, 0.0, 0.0, 10.0), Some(1.0));
        assert_eq!(kappa(0.0, 10.0, 10.0, 0.0), Some(-1.0));
        // Agreement exactly at chance is 0.
        assert!(kappa(25.0, 25.0, 25.0, 25.0).unwrap().abs() < 1e-12);
        // Everyone said yes every time: kappa is not defined.
        assert_eq!(kappa(12.0, 0.0, 0.0, 0.0), None);
        assert_eq!(kappa(0.0, 0.0, 0.0, 0.0), None);
    }

    fn project(dir: &Path, name: &str, folder: &str) -> Project {
        let p = Project::open(&dir.join(name)).unwrap();
        p.db.execute_batch(&format!("INSERT INTO deployments (id, name, folder, serial) VALUES (1, 'A', '{folder}', '');
            INSERT INTO files (id, deployment_id, path, start_local) VALUES (1, 1, '{folder}/a.wav', '2025-06-01 05:00:00');
            INSERT INTO species (id, scientific, common) VALUES (1, 'Turdus merula', 'Eurasian Blackbird'), (2, 'Erithacus rubecula', 'European Robin');")).unwrap();
        // Six blackbird detections and two robin detections, same in both copies.
        for k in 0..8 { p.db.execute("INSERT INTO detections (file_id, model, start_s, end_s, species, confidence, in_range) VALUES (1, 'birdnet-2.4', ?1, ?1 + 3, ?2, 0.8, 1)",
            params![k as f64 * 3.0, if k < 6 { 1 } else { 2 }]).unwrap(); }
        p
    }

    #[test]
    fn two_reviewers() {
        let dir = tempfile::tempdir().unwrap();
        let me = project(dir.path(), "me.wlnw", "/data/site");
        let other = project(dir.path(), "other.wlnw", "D:\\copy\\site");
        let ids = |p: &Project| -> Vec<i64> { p.db.prepare("SELECT id FROM detections ORDER BY start_s").unwrap().query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect() };
        let (mi, oi) = (ids(&me), ids(&other));
        // Blackbird: me y y y n n u, other y y n n y y. Robin: me y n, other y n.
        for (k, v) in ["yes", "yes", "yes", "no", "no", "unsure", "yes", "no"].iter().enumerate() { crate::review::set_verdict(&me, mi[k], Some(v)).unwrap(); }
        for (k, v) in ["yes", "yes", "no", "no", "yes", "yes", "yes", "no"].iter().enumerate() { crate::review::set_verdict(&other, oi[k], Some(v)).unwrap(); }
        let f = dir.path().join("other.wlnr");
        assert_eq!(export(&other, &f, Some("Priya")).unwrap(), 8);
        // The other copy lives in another folder: matched by file name.
        let l = load(&me, &f, None).unwrap();
        assert_eq!((l.reviewer.as_str(), l.matched, l.unmatched), ("Priya", 8, 0));
        // My own answers are untouched.
        let mine: i64 = me.db.query_row("SELECT COUNT(*) FROM reviews WHERE verdict = 'unsure'", [], |r| r.get(0)).unwrap();
        assert_eq!(mine, 1);

        let r = report(&me, None).unwrap();
        assert_eq!(r.reviewer.as_deref(), Some("Priya"));
        let bb = r.species.iter().find(|s| s.scientific == "Turdus merula").unwrap();
        // Five yes/no pairs: yy 2, yn 1 (me yes, other no), ny 1, nn 1. po 0.6, pe 0.52, kappa 1/6.
        assert_eq!((bb.both, bb.yes_no, bb.yy, bb.yn, bb.ny, bb.nn), (6, 5, 2, 1, 1, 1));
        assert!((bb.percent.unwrap() - 60.0).abs() < 1e-9);
        assert!((bb.kappa.unwrap() - 1.0 / 6.0).abs() < 1e-9);
        let rb = r.species.iter().find(|s| s.scientific == "Erithacus rubecula").unwrap();
        assert_eq!((rb.percent, rb.kappa), (Some(100.0), Some(1.0)));
        assert_eq!((r.overall.both, r.overall.yes_no), (8, 7));
        // Disagreements: two yes/no splits and the unsure.
        assert_eq!(r.disagreements.len(), 3);

        // Settle one: the final answer is written, the figures keep the first answers.
        let d = r.disagreements.iter().find(|d| d.detection.verdict.as_deref() == Some("yes")).unwrap();
        settle(&me, "Priya", d.detection.id, "no").unwrap();
        let r2 = report(&me, Some("Priya")).unwrap();
        assert_eq!((r2.disagreements.len(), r2.settled), (2, 1));
        assert!((r2.species.iter().find(|s| s.scientific == "Turdus merula").unwrap().kappa.unwrap() - 1.0 / 6.0).abs() < 1e-9);
        let v: String = me.db.query_row("SELECT verdict FROM reviews WHERE detection_id = ?1", [d.detection.id], |r| r.get(0)).unwrap();
        assert_eq!(v, "no");
        // Loading again keeps the settled state.
        load(&me, &f, None).unwrap();
        assert_eq!(report(&me, None).unwrap().settled, 1);
        assert!(settle(&me, "Nobody", d.detection.id, "yes").is_err());
    }
}
