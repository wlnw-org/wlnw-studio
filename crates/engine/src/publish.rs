//! Publishing to a Wildlife Network dataset with its upload key (https://wlnw.org/datasets): rows in chunks, then the
//! site's own checks, then submit. Re-sending is safe: the site keeps one record per site, species and time.
use crate::store::Project;
use anyhow::{bail, Result};
use serde::Serialize;

pub const API: &str = "https://wlnw.org/api/v1/datasets";
const CHUNK: usize = 2000;
/** Records one dataset can hold on wlnw.org (DATASET_LIMITS.MAX_RECORDS in the site's src/lib/datasets.ts). */
const MAX_RECORDS: usize = 300_000;

#[derive(Debug, Default, Serialize)]
pub struct PublishReport {
    pub sent: usize, pub added: i64, pub duplicates: i64, pub left_out: i64, pub status: Option<String>,
    /// Accepted detections and images not sent because their recorder or camera has no place, or the file no time.
    pub no_place_or_time: usize,
    /// Accepted detections not sent because they came live from a Wildlife Network Base, which already uploads them.
    pub from_base: usize,
    /// Why the dataset details (title, people, licence) could not be updated on the site, if they could not.
    pub details_error: Option<String>,
}

fn post(dataset: &str, key: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    if dataset.is_empty() || dataset.len() > 64 || !dataset.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        anyhow::bail!("That dataset id does not look right: copy it from the dataset's page on wlnw.org.");
    }
    let r = ureq::post(&format!("{API}/{dataset}"))
        .set("authorization", &format!("Bearer {key}"))
        .set("user-agent", concat!("WildlifeNetworkStudio/", env!("CARGO_PKG_VERSION")))
        .send_json(body);
    match r {
        Ok(res) => Ok(res.into_json()?),
        Err(ureq::Error::Status(code, res)) => {
            let v: serde_json::Value = res.into_json().unwrap_or_default();
            bail!("Wildlife Network said {code}: {}", v["error"].as_str().unwrap_or("no message"))
        }
        Err(e) => bail!("could not reach Wildlife Network: {e}"),
    }
}

/// The "About this dataset" details from Studio, sent as a partial update (the site keeps fields not sent).
fn details_body(p: &Project) -> Option<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(&p.setting("dataset").ok()??).ok()?;
    let people: Vec<serde_json::Value> = v["people"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(|n| serde_json::json!({ "name": n })).collect()).unwrap_or_default();
    Some(serde_json::json!({ "action": "update", "title": v["title"], "institution": v["institution"], "description": v["description"],
        "license": v["license"], "people": people }))
}

/// Accepted detections to send, and how many were held back because they came live from a Wildlife Network Base: the Base
/// already sends its detections to Wildlife Network, so publishing them again would count them twice. Live BirdNET-Go
/// stations that are not a Base stay publishable. Exports are not affected.
pub fn publishable(p: &Project, accepted: Vec<crate::store::Detection>) -> Result<(Vec<crate::store::Detection>, usize)> {
    let mut st = p.db.prepare("SELECT id FROM deployments WHERE folder LIKE ?1")?;
    let base: std::collections::HashSet<i64> = st.query_map([format!("{}base:%", crate::live::PREFIX)], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let before = accepted.len();
    let keep: Vec<_> = accepted.into_iter().filter(|d| !base.contains(&d.deployment_id)).collect();
    let held = before - keep.len();
    Ok((keep, held))
}

pub fn publish(p: &Project, dataset: &str, key: &str, submit: bool, mut on_progress: impl FnMut(usize, usize)) -> Result<PublishReport> {
    let (accepted, from_base) = publishable(p, p.accepted()?)?;
    let total_audio = accepted.len();
    let rows: Vec<serde_json::Value> = accepted.into_iter().filter_map(|d| {
        let (t, lat, lon) = (d.utc?, d.lat?, d.lon?);
        Some(serde_json::json!({ "site": d.site, "lat": lat, "lon": lon, "ts": t.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "name": d.scientific, "confidence": d.confidence, "count": 1,
            "file": std::path::Path::new(&d.path).file_name().map(|n| n.to_string_lossy().to_string()) }))
    }).collect();
    // Camera-trap sightings: one row per image and taxon with the number of boxes; never images with people, never vehicles.
    let images = crate::images::accepted(p)?;
    let skipped_audio = total_audio - rows.len();
    let mut rows = rows;
    let per_image = crate::images::per_image(&images);
    let total_images = per_image.len();
    let before = rows.len();
    rows.extend(per_image.into_iter().filter_map(|(o, count)| {
        let (t, lat, lon) = (o.utc?, o.lat?, o.lon?);
        Some(serde_json::json!({ "site": o.site, "lat": lat, "lon": lon, "ts": t.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "name": o.scientific, "confidence": o.score, "count": count,
            "file": std::path::Path::new(&o.path).file_name().map(|n| n.to_string_lossy().to_string()) }))
    }));
    // The site holds up to MAX_RECORDS per dataset: stop before sending anything rather than failing halfway.
    if rows.len() > MAX_RECORDS {
        bail!("This project has {} records to send, more than the {} a Wildlife Network dataset can hold. Make a second dataset and publish part of the project to each (for example by recorder), or export a Darwin Core Archive instead.", rows.len(), MAX_RECORDS);
    }
    let skipped = skipped_audio + (total_images - (rows.len() - before));
    let mut rep = PublishReport { sent: rows.len(), no_place_or_time: skipped, from_base, ..Default::default() };
    // Details first, so the dataset page shows the right title and credits. Not fatal: records still go.
    if let Some(body) = details_body(p) {
        if let Err(e) = post(dataset, key, body) { rep.details_error = Some(e.to_string()); }
    }
    for (i, part) in rows.chunks(CHUNK).enumerate() {
        let v = post(dataset, key, serde_json::json!({ "action": "records", "rows": part }))?;
        rep.added += v["added"].as_i64().unwrap_or(0);
        rep.duplicates += v["duplicates"].as_i64().unwrap_or(0);
        rep.left_out += v["unmatched"].as_i64().unwrap_or(0);
        on_progress(((i + 1) * CHUNK).min(rows.len()), rows.len());
    }
    if submit && rep.sent > 0 {
        let v = post(dataset, key, serde_json::json!({ "action": "submit" }))?;
        rep.status = v["status"].as_str().map(String::from);
    }
    p.set_setting("dataset_id", dataset)?;
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{store, LiveReport, Row, Station};

    #[test]
    fn base_detections_are_never_published() {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(&dir.path().join("P.wlnw")).unwrap();
        let row = |id: &str| Row { id: id.into(), scientific: "Erithacus rubecula".into(), common: "European Robin".into(), confidence: 0.9,
            local: chrono::NaiveDateTime::parse_from_str("2026-11-18 06:00:00", "%Y-%m-%d %H:%M:%S").unwrap(), tz_offset_min: Some(0), clip_ext: Some("wav".into()) };
        let station = |key: &str, kind: &str| Station { key: key.into(), kind: kind.into(), name: key.into(), address: "http://x".into(), api: "http://x:8080".into(),
            lat: Some(51.5), lon: Some(-0.1), last: None, imported_at: None };
        store(&p, &mut station("base:abc", "base"), &[row("1"), row("2")], &mut LiveReport::default()).unwrap();
        store(&p, &mut station("bng:x:8080", "birdnet-go"), &[row("3")], &mut LiveReport::default()).unwrap();
        let accepted = p.accepted().unwrap();
        assert_eq!(accepted.len(), 3);
        let (keep, held) = publishable(&p, accepted).unwrap();
        assert_eq!((keep.len(), held), (1, 2));
        assert_eq!(keep[0].site, "bng:x:8080");
        // Exports still carry every accepted detection.
        assert_eq!(p.accepted().unwrap().len(), 3);
    }
}
