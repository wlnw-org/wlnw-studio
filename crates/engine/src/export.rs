//! Exports: Darwin Core Archive for GBIF (IPT or direct), WDX for Wildlife Network and other tools, Raven selection tables,
//! Audacity labels, and plain CSV. All use the accepted detections (in range, at or above each species' threshold,
//! not rejected), so what is exported is what was reviewed.
use crate::store::{Detection, Project};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatasetInfo {
    pub title: String, pub institution: Option<String>, pub people: Vec<String>, pub license: String,
    pub description: Option<String>, pub contact_email: Option<String>,
}

pub fn info(p: &Project) -> DatasetInfo {
    p.setting("dataset").ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| DatasetInfo {
        title: "Acoustic detections".into(), license: "CC-BY-4.0".into(), ..Default::default()
    })
}

fn q(v: &str) -> String { if v.contains([',', '"', '\n']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v.to_string() } }
fn tsv(v: &str) -> String { v.replace(['\t', '\n', '\r'], " ") }
const LICENSE_URL: &[(&str, &str)] = &[
    ("CC0-1.0", "http://creativecommons.org/publicdomain/zero/1.0/legalcode"),
    ("CC-BY-4.0", "http://creativecommons.org/licenses/by/4.0/legalcode"),
    ("CC-BY-NC-4.0", "http://creativecommons.org/licenses/by-nc/4.0/legalcode"),
];
fn license_url(l: &str) -> &'static str { LICENSE_URL.iter().find(|(k, _)| *k == l).map_or(LICENSE_URL[1].1, |x| x.1) }
fn utc(d: &Detection) -> String { d.utc.map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string()).unwrap_or_default() }
/// The model's name as written in exports ("BirdNET 2.4", "Perch 2.0", "BatDetect2 UK", "Wildlife Network Classifier").
pub fn model_name(id: &str) -> String {
    match id { "birdnet-2.4" => "BirdNET 2.4".into(), "perch-2.0" => "Perch 2.0".into(), "batdetect2-uk" => "BatDetect2 UK".into(), x if x.starts_with("wnc") => "Wildlife Network Classifier".into(),
        // Imported results: the tool's name with no space, so WDX's classifier name/version split keeps it whole.
        "import:birdnet-analyzer" => "BirdNET-Analyzer".into(), "import:birdnet-pi" => "BirdNET-Pi/BirdNET-Go".into(), "import:kaleidoscope" => "Kaleidoscope".into(), x => x.into() }
}
/// How a detection's species threshold was set, written next to every exported record so users of the data can judge it.
pub struct Basis { pub threshold: f64, pub fitted: bool, pub reviewed: i64, pub precision: Option<f64> }

pub fn basis(p: &Project, species: i64, model: &str) -> Basis {
    use rusqlite::OptionalExtension;
    let row: Option<(f64, String, Option<f64>, i64)> = p.db.query_row("SELECT threshold, source, precision, reviewed FROM thresholds WHERE species = ?1 AND model = ?2",
        rusqlite::params![species, model], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional().ok().flatten();
    match row {
        Some((t, src, prec, n)) => Basis { threshold: t, fitted: src == "fitted", reviewed: n, precision: prec },
        None => Basis { threshold: p.default_threshold(model), fitted: false, reviewed: 0, precision: None },
    }
}

/// "species threshold 0.62 fitted from 14 reviewed clips (estimated precision 0.93)" or "default threshold 0.50 (species not reviewed)".
pub fn basis_text(b: &Basis) -> String {
    if b.fitted {
        format!("species threshold {:.2} fitted from {} reviewed clips{}", b.threshold, b.reviewed,
            b.precision.map(|x| format!(" (estimated precision {x:.2})")).unwrap_or_default())
    } else { format!("default threshold {:.2} (species not reviewed)", b.threshold) }
}

/// The citations for camera-trap results: SpeciesNet names the animal on MegaDetector's box.
fn image_reference() -> String { format!("{} | {}", crate::credits::reference("speciesnet"), crate::credits::reference("megadetector")) }

/// Writes CITATION.txt (who made the models in these results and how to cite them) at `path`.
fn write_citation(path: &Path, models: &[&str]) -> Result<()> { std::fs::write(path, crate::credits::citation_file(models))?; Ok(()) }

pub const STUDIO: &str = concat!("Wildlife Network Studio ", env!("CARGO_PKG_VERSION"));

fn status(d: &Detection) -> &'static str { if d.verdict.as_deref() == Some("yes") { "verified by expert" } else { "unverified" } }

pub fn csv(p: &Project, out: &Path) -> Result<usize> {
    let rows = p.accepted()?;
    let mut f = std::io::BufWriter::new(std::fs::File::create(out)?);
    writeln!(f, "site,latitude,longitude,file,start_s,end_s,utc,scientific_name,common_name,model,confidence,review,threshold,threshold_basis,reviewed_clips,estimated_precision,software,media,bbox_x,bbox_y,bbox_width,bbox_height,model_reference")?;
    let mut cache: std::collections::HashMap<(i64, String), Basis> = std::collections::HashMap::new();
    for d in &rows {
        let b = cache.entry((d.label, d.model.clone())).or_insert_with(|| basis(p, d.label, &d.model));
        writeln!(f, "{},{},{},{},{:.1},{:.1},{},{},{},{},{:.4},{},{:.4},{},{},{},{},audio,,,,,{}", q(&d.site), d.lat.map_or(String::new(), |v| v.to_string()), d.lon.map_or(String::new(), |v| v.to_string()),
            q(&d.path), d.start_s, d.end_s, utc(d), q(&d.scientific), q(&d.common), model_name(&d.model), d.confidence, d.verdict.as_deref().unwrap_or(""),
            b.threshold, if b.fitted { "fitted" } else { "default" }, b.reviewed, b.precision.map(|x| format!("{x:.3}")).unwrap_or_default(), STUDIO, q(crate::credits::reference(&d.model)))?;
    }
    // Camera-trap photos: one row per accepted animal box (people never), so a photo-only project still gets a useful CSV.
    let boxes = crate::images::accepted(p)?;
    for o in &boxes {
        let (name, ver) = crate::images::model_label(&o.model);
        writeln!(f, "{},{},{},{},,,{},{},{},{},{:.4},{},,,,,{},image,{:.4},{:.4},{:.4},{:.4},{}", q(&o.site), o.lat.map_or(String::new(), |v| v.to_string()), o.lon.map_or(String::new(), |v| v.to_string()),
            q(&o.path), o.utc.map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string()).unwrap_or_default(), q(&o.scientific), q(&o.common), q(&format!("{name} {ver}")),
            o.score, o.verdict.as_deref().unwrap_or(""), STUDIO, o.bbox[0], o.bbox[1], o.bbox[2], o.bbox[3], q(&image_reference()))?;
    }
    Ok(rows.len() + boxes.len())
}

/// One Raven selection table and one Audacity label file per recording, next to each other in `dir`.
pub fn raven_audacity(p: &Project, dir: &Path) -> Result<usize> {
    std::fs::create_dir_all(dir)?;
    let rows = p.accepted()?;
    let mut by_file: BTreeMap<&str, Vec<&Detection>> = BTreeMap::new();
    for d in &rows { by_file.entry(&d.path).or_default().push(d); }
    for (path, dets) in &by_file {
        let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let mut r = std::fs::File::create(dir.join(format!("{stem}.Table.1.selections.txt")))?;
        writeln!(r, "Selection\tView\tChannel\tBegin Time (s)\tEnd Time (s)\tLow Freq (Hz)\tHigh Freq (Hz)\tSpecies Code\tCommon Name\tConfidence\tBegin Path\tFile Offset (s)")?;
        let mut a = std::fs::File::create(dir.join(format!("{stem}.labels.txt")))?;
        for (i, d) in dets.iter().enumerate() {
            writeln!(r, "{}\tSpectrogram 1\t1\t{:.1}\t{:.1}\t0\t15000\t{}\t{}\t{:.4}\t{}\t{:.1}", i + 1, d.start_s, d.end_s, tsv(&d.scientific), tsv(&d.common), d.confidence, tsv(path), d.start_s)?;
            writeln!(a, "{:.1}\t{:.1}\t{} ({:.2})", d.start_s, d.end_s, if d.common.is_empty() { &d.scientific } else { &d.common }, d.confidence)?;
        }
    }
    // Raven and Audacity tables have no place for credits: they go in a CITATION.txt next to them.
    if !rows.is_empty() { write_citation(&dir.join("CITATION.txt"), &rows.iter().map(|d| d.model.as_str()).collect::<Vec<_>>())?; }
    Ok(rows.len())
}

/// A Kaleidoscope-style id.csv: one row per accepted detection; MANUAL ID holds the species for detections reviewed yes.
pub fn kaleidoscope(p: &Project, out: &Path) -> Result<usize> {
    let rows = p.accepted()?;
    let mut f = std::io::BufWriter::new(std::fs::File::create(out)?);
    writeln!(f, "FOLDER,IN FILE,OFFSET,DURATION,AUTO ID*,MATCH RATIO,MANUAL ID")?;
    for d in &rows {
        let path = Path::new(&d.path);
        let folder = path.parent().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        let file = path.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        writeln!(f, "{},{},{:.2},{:.2},{},{:.4},{}", q(&folder), q(&file), d.start_s, d.end_s - d.start_s, q(&d.scientific), d.confidence,
            if d.verdict.as_deref() == Some("yes") { q(&d.scientific) } else { String::new() })?;
    }
    // id.csv has no place for credits: they go in "<file name>.CITATION.txt" next to it.
    if !rows.is_empty() {
        let name = out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "id.csv".into());
        write_citation(&out.with_file_name(format!("{name}.CITATION.txt")), &rows.iter().map(|d| d.model.as_str()).collect::<Vec<_>>())?;
    }
    Ok(rows.len())
}

/// WDX v0.4 events, one per line (https://github.com/arunrajiah/wildlife-detection-exchange).
pub fn wdx(p: &Project, out: &Path) -> Result<usize> {
    let rows = p.accepted()?;
    let i = info(p);
    let ds = p.setting("dataset_id")?.unwrap_or_else(|| "studio".into());
    let mut f = std::io::BufWriter::new(std::fs::File::create(out)?);
    for d in &rows {
        let Some(t) = d.utc else { continue };
        let ev = serde_json::json!({
            "wdx": "0.4", "eventId": format!("wlnw-studio:{}:{}", ds, d.id),
            "eventStart": t.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "eventEnd": (t + chrono::Duration::milliseconds(((d.end_s - d.start_s) * 1000.0) as i64)).format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "deployment": { "deploymentId": format!("{}:{}", ds, d.deployment_id), "name": d.site, "latitude": d.lat, "longitude": d.lon, "sensorType": "acoustic-recorder" },
            "detection": { "scientificName": d.scientific, "vernacularName": d.common, "taxonRank": "species", "confidence": (d.confidence * 10000.0).round() / 10000.0,
                "classifier": { "name": model_name(&d.model).rsplit_once(' ').map_or(model_name(&d.model), |x| x.0.to_string()), "version": model_name(&d.model).rsplit_once(' ').map_or(String::new(), |x| x.1.to_string()),
                    "reference": crate::credits::for_model(&d.model).map(|c| c.paper_url) } },
            "media": { "mediaType": "audio", "fileName": Path::new(&d.path).file_name().map(|n| n.to_string_lossy().to_string()), "startOffsetSeconds": d.start_s, "durationSeconds": d.end_s - d.start_s },
            "review": { "status": if d.verdict.as_deref() == Some("yes") { "confirmed" } else { "unreviewed" } },
            "source": { "system": "wlnw-studio", "systemVersion": env!("CARGO_PKG_VERSION"), "sourceRecordId": d.id.to_string() },
            "license": i.license,
            "dataset": { "datasetId": ds, "title": i.title, "rightsHolder": i.institution, "recordedBy": i.people },
        });
        if d.lat.is_none() { continue; }
        writeln!(f, "{ev}")?;
    }
    // Camera-trap boxes: one event per box (WDX allows one region per event); never people or vehicles.
    let obs = crate::images::accepted(p)?;
    let mut n_img = 0;
    for o in &obs {
        let (Some(t), Some(lat), Some(lon)) = (o.utc, o.lat, o.lon) else { continue };
        let (name, ver) = crate::images::model_label(&o.model);
        let rank = if o.rank == "kingdom" { "unranked" } else { o.rank.as_str() };
        let ev = serde_json::json!({
            "wdx": "0.4", "eventId": format!("wlnw-studio:{}:img{}", ds, o.box_id),
            "eventStart": t.format("%Y-%m-%dT%H:%M:%SZ").to_string(), "eventEnd": t.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "deployment": { "deploymentId": format!("{}:{}", ds, o.deployment_id), "name": o.site, "latitude": lat, "longitude": lon, "sensorType": "camera-trap" },
            "detection": { "scientificName": o.scientific, "vernacularName": o.common, "taxonRank": rank, "individualCount": 1,
                "confidence": (o.score * 10000.0).round() / 10000.0, "classifier": { "name": name, "version": ver, "reference": crate::credits::reference("speciesnet").rsplit(' ').next() },
                "pipeline": [{ "role": "detector", "name": "MegaDetector", "version": "6 MIT YOLOv9" }, { "role": "classifier", "name": name, "version": ver }] },
            "media": { "mediaType": "image", "fileName": Path::new(&o.path).file_name().map(|n| n.to_string_lossy().to_string()),
                "region": { "type": "box", "units": "normalized", "coordinates": o.bbox.iter().map(|v| (v * 10000.0).round() / 10000.0).collect::<Vec<_>>(),
                    "imageWidth": o.width, "imageHeight": o.height } },
            "review": { "status": if o.verdict.as_deref() == Some("yes") { "confirmed" } else { "unreviewed" } },
            "source": { "system": "wlnw-studio", "systemVersion": env!("CARGO_PKG_VERSION"), "sourceRecordId": format!("img{}", o.box_id) },
            "license": i.license,
            "dataset": { "datasetId": ds, "title": i.title, "rightsHolder": i.institution, "recordedBy": i.people },
        });
        writeln!(f, "{ev}")?;
        n_img += 1;
    }
    Ok(rows.len() + n_img)
}

/// A Darwin Core Archive (occurrence core) ready for an IPT or GBIF: occurrence.txt, meta.xml and a minimal eml.xml.
pub fn dwca(p: &Project, out: &Path) -> Result<usize> {
    let rows = p.accepted()?;
    let i = info(p);
    let ds = p.setting("dataset_id")?.unwrap_or_else(|| "wlnw-studio".into());
    let cols = ["occurrenceID", "basisOfRecord", "eventDate", "scientificName", "vernacularName", "decimalLatitude", "decimalLongitude",
        "coordinateUncertaintyInMeters", "locationID", "recordedBy", "institutionCode", "datasetName", "license", "identifiedBy",
        "identificationVerificationStatus", "identificationRemarks", "associatedMedia", "occurrenceStatus", "identificationReferences"];
    let images = crate::images::accepted(p)?;
    let has_images = !images.is_empty();
    // Taxon rank and counts only matter for camera-trap rows (sound rows stay as they were when there are none).
    let mut cols = cols.to_vec();
    if has_images { cols.extend(["taxonRank", "individualCount"]); }
    let mut occ = String::new();
    occ.push_str(&cols.join("\t"));
    occ.push('\n');
    let mut n = 0;
    let mut cache: std::collections::HashMap<(i64, String), Basis> = std::collections::HashMap::new();
    for d in &rows {
        let (Some(_), Some(lat), Some(lon)) = (d.utc, d.lat, d.lon) else { continue };
        let line = [format!("{ds}:{}", d.id), "MachineObservation".into(), utc(d), d.scientific.clone(), d.common.clone(), format!("{lat:.5}"), format!("{lon:.5}"),
            "100".into(), d.site.clone(), i.people.join(" | "), i.institution.clone().unwrap_or_default(), i.title.clone(), license_url(&i.license).into(),
            format!("{}{}", model_name(&d.model), if d.verdict.as_deref() == Some("yes") { " | confirmed by reviewer" } else { "" }), status(d).into(),
            format!("{} score {:.4}; {}; clip {:.1} to {:.1} s; {STUDIO}", model_name(&d.model), d.confidence,
                if d.verdict.as_deref() == Some("yes") { "confirmed by a reviewer".to_string() } else { basis_text(cache.entry((d.label, d.model.clone())).or_insert_with(|| basis(p, d.label, &d.model))) },
                d.start_s, d.end_s),
            Path::new(&d.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), "present".into(), crate::credits::reference(&d.model).into()];
        let mut line = line.to_vec();
        if has_images { line.extend(["species".to_string(), String::new()]); }
        occ.push_str(&line.iter().map(|v| tsv(v)).collect::<Vec<_>>().join("\t"));
        occ.push('\n');
        n += 1;
    }
    // Camera-trap sightings: one occurrence per image and taxon, with how many boxes; never people or vehicles.
    for (o, count) in crate::images::per_image(&images) {
        let (Some(t), Some(lat), Some(lon)) = (o.utc, o.lat, o.lon) else { continue };
        let (name, ver) = crate::images::model_label(&o.model);
        let line = [format!("{ds}:img{}:{}", o.image_id, o.species), "MachineObservation".into(), t.format("%Y-%m-%dT%H:%M:%SZ").to_string(), o.scientific.clone(), o.common.clone(),
            format!("{lat:.5}"), format!("{lon:.5}"), "100".into(), o.site.clone(), i.people.join(" | "), i.institution.clone().unwrap_or_default(), i.title.clone(), license_url(&i.license).into(),
            format!("{name} {ver} with MegaDetector v6{}", if o.verdict.as_deref() == Some("yes") { " | confirmed by reviewer" } else { "" }),
            if o.verdict.as_deref() == Some("yes") { "verified by expert" } else { "unverified" }.into(),
            format!("camera trap image; score {:.4}", o.score), Path::new(&o.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), "present".into(),
            image_reference(), o.rank.clone(), count.to_string()];
        occ.push_str(&line.iter().map(|v| tsv(v)).collect::<Vec<_>>().join("\t"));
        occ.push('\n');
        n += 1;
    }
    let fields: String = cols.iter().enumerate().skip(1).map(|(k, c)| format!("    <field index=\"{k}\" term=\"http://rs.tdwg.org/dwc/terms/{c}\"/>\n").replace("dwc/terms/license", "purl.org/dc/terms/license").replace("http://rs.tdwg.org/purl.org", "http://purl.org")).collect();
    let meta = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<archive xmlns=\"http://rs.tdwg.org/dwc/text/\" metadata=\"eml.xml\">\n  <core encoding=\"UTF-8\" fieldsTerminatedBy=\"\\t\" linesTerminatedBy=\"\\n\" fieldsEnclosedBy=\"\" ignoreHeaderLines=\"1\" rowType=\"http://rs.tdwg.org/dwc/terms/Occurrence\">\n    <files><location>occurrence.txt</location></files>\n    <id index=\"0\"/>\n{fields}  </core>\n</archive>\n");
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let creators: String = i.people.iter().map(|p| format!("    <creator><individualName><surName>{}</surName></individualName></creator>\n", esc(p))).collect();
    // One method step per model with the citation its authors ask for.
    let mut used: Vec<&str> = rows.iter().map(|d| d.model.as_str()).collect();
    if has_images { used.extend(["speciesnet", "megadetector"]); }
    let cites: String = crate::credits::CREDITS.iter().filter(|c| used.iter().any(|m| crate::credits::for_model(m).is_some_and(|x| x.id == c.id)))
        .map(|c| format!("<methodStep><description><para>{} by {}, licence {}. Cite {}</para></description></methodStep>", esc(c.name), esc(c.by), esc(c.licence), esc(c.citation))).collect();
    let eml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<eml:eml xmlns:eml=\"https://eml.ecoinformatics.org/eml-2.2.0\" packageId=\"{}\" system=\"wlnw-studio\">\n  <dataset>\n    <title>{}</title>\n{creators}{}    <abstract><para>{}</para></abstract>\n    <intellectualRights><para>{}</para></intellectualRights>\n    <methods><methodStep><description><para>Automatic identification in Wildlife Network Studio {} ({}); per-species thresholds fitted from expert review where reviewed.</para></description></methodStep>{cites}</methods>\n  </dataset>\n</eml:eml>\n",
        esc(&ds), esc(&i.title), i.institution.as_ref().map(|x| format!("    <organization>{}</organization>\n", esc(x))).unwrap_or_default(), esc(i.description.as_deref().unwrap_or("")), license_url(&i.license), env!("CARGO_PKG_VERSION"), { let mut m: Vec<String> = rows.iter().map(|d| model_name(&d.model)).collect(); if has_images { m.push("SpeciesNet 4.0.3a with MegaDetector v6 MIT".into()); } m.sort(); m.dedup(); m.join(", ") });
    let mut z = zip::ZipWriter::new(std::fs::File::create(out)?);
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in [("occurrence.txt", occ), ("meta.xml", meta), ("eml.xml", eml)] {
        z.start_file(name, o)?;
        z.write_all(body.as_bytes())?;
    }
    z.finish()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bat detections are credited to BatDetect2 (paper and repository) in every export that holds them.
    #[test]
    fn bat_exports_cite_batdetect2() {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::open(&dir.path().join("t.wlnw")).unwrap();
        p.db.execute_batch("INSERT INTO deployments (id, name, folder, serial, lat, lon) VALUES (1, 'Pond', '/a', 'A', 51.5, -0.1);
            INSERT INTO files (id, deployment_id, path, start_local, duration_s) VALUES (1, 1, '/a/bat.wav', '2026-06-01 22:00:00', 60);
            INSERT INTO file_models (file_id, model, status, processed_at) VALUES (1, 'batdetect2-uk', 'done', '');
            INSERT INTO species (id, scientific, common) VALUES (1, 'Pipistrellus pipistrellus', 'Common Pipistrelle');
            INSERT INTO detections (file_id, model, start_s, end_s, species, confidence, in_range) VALUES (1, 'batdetect2-uk', 1.2, 1.25, 1, 0.95, 1);").unwrap();
        let doi = "https://doi.org/10.1101/2022.12.14.520490";
        let d = dir.path();
        assert_eq!(csv(&p, &d.join("all.csv")).unwrap(), 1);
        let c = std::fs::read_to_string(d.join("all.csv")).unwrap();
        assert!(c.lines().next().unwrap().ends_with(",model_reference") && c.contains(",BatDetect2 UK,") && c.contains(doi));
        raven_audacity(&p, &d.join("raven")).unwrap();
        let cite = std::fs::read_to_string(d.join("raven/CITATION.txt")).unwrap();
        assert!(cite.contains(doi) && cite.contains("https://github.com/macaodha/batdetect2") && cite.contains("CC BY-NC 4.0"));
        kaleidoscope(&p, &d.join("id.csv")).unwrap();
        assert!(std::fs::read_to_string(d.join("id.csv.CITATION.txt")).unwrap().contains(doi));
        wdx(&p, &d.join("out.wdx")).unwrap();
        let w = std::fs::read_to_string(d.join("out.wdx")).unwrap();
        assert!(w.contains("\"name\":\"BatDetect2\"") && w.contains(doi));
        dwca(&p, &d.join("dwca.zip")).unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(d.join("dwca.zip")).unwrap()).unwrap();
        let mut read = |n: &str| { let mut s = String::new(); std::io::Read::read_to_string(&mut z.by_name(n).unwrap(), &mut s).unwrap(); s };
        let (occ, meta, eml) = (read("occurrence.txt"), read("meta.xml"), read("eml.xml"));
        assert!(occ.lines().next().unwrap().contains("identificationReferences") && occ.contains(doi));
        assert!(meta.contains("dwc/terms/identificationReferences"));
        assert!(eml.contains("BatDetect2 (UK model) by Oisin Mac Aodha") && eml.contains(doi));
    }
    #[test]
    fn basis_wording() {
        assert_eq!(basis_text(&Basis { threshold: 0.62, fitted: true, reviewed: 14, precision: Some(0.931) }),
            "species threshold 0.62 fitted from 14 reviewed clips (estimated precision 0.93)");
        assert_eq!(basis_text(&Basis { threshold: 0.5, fitted: false, reviewed: 0, precision: None }), "default threshold 0.50 (species not reviewed)");
    }
}
