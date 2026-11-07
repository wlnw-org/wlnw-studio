//! Publishing to a Wildlife Network dataset with its upload key (https://wlnw.org/datasets): rows in chunks, then the
//! site's own checks, then submit. Re-sending is safe: the site keeps one record per site, species and time.
use crate::store::Project;
use anyhow::{bail, Result};
use serde::Serialize;

pub const API: &str = "https://wlnw.org/api/v1/datasets";
const CHUNK: usize = 2000;

#[derive(Debug, Default, Serialize)]
pub struct PublishReport { pub sent: usize, pub added: i64, pub duplicates: i64, pub left_out: i64, pub status: Option<String> }

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

pub fn publish(p: &Project, dataset: &str, key: &str, submit: bool, mut on_progress: impl FnMut(usize, usize)) -> Result<PublishReport> {
    let rows: Vec<serde_json::Value> = p.accepted()?.into_iter().filter_map(|d| {
        let (t, lat, lon) = (d.utc?, d.lat?, d.lon?);
        Some(serde_json::json!({ "site": d.site, "lat": lat, "lon": lon, "ts": t.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "name": d.scientific, "confidence": d.confidence, "count": 1,
            "file": std::path::Path::new(&d.path).file_name().map(|n| n.to_string_lossy().to_string()) }))
    }).collect();
    let mut rep = PublishReport { sent: rows.len(), ..Default::default() };
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
