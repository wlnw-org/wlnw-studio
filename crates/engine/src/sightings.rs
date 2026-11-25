//! One animal in a video is one sighting. The boxes frames.rs found are joined from frame to frame (same kind of thing,
//! overlapping or close to where it was, seen again within a few seconds); each track becomes a sighting with the name
//! most of its frames agree on, its sharpest frame kept as a still and a short clip cut around it. A sighting is stored
//! as a camera-trap image (the still) that points back to its video, so review, thresholds and every export treat it like
//! a photo, with the video's file name and the sighting's start and end added.
use crate::camtrap;
use crate::store::Project;
use anyhow::Result;
use rusqlite::{params, Transaction};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// A box seen in one frame, as frames.rs stored it.
#[derive(Debug, Clone)]
pub struct Seen { pub t: f64, pub category: String, pub conf: f64, pub bbox: [f64; 4], pub kind: String, pub label: Option<String>, pub score: f64, pub source: Option<String>, pub top: String, pub sharp: f64 }

/// A track: the boxes of one animal (or person, or vehicle), in time order.
#[derive(Debug, Clone)]
pub struct Track { pub seen: Vec<Seen> }

/// How long an animal may be out of sight (hidden, or missed by the detector) and still be the same animal.
pub const MAX_GAP_S: f64 = 3.0;
/// Boxes this much overlapping (intersection over union) are the same animal...
pub const MIN_IOU: f64 = 0.1;
/// ...and so are boxes whose centres are this close, as a fraction of the frame (animals move far in a second).
pub const MAX_JUMP: f64 = 0.2;
/// Boxes fainter than this are left out (SpeciesNet does not name them either)...
pub const MIN_BOX_CONF: f64 = 0.1;
/// ...and a track whose most confident box is below this is not a sighting (MegaDetector's usual threshold).
pub const MIN_TRACK_CONF: f64 = 0.2;
/// A box mostly inside a more confident box of the same kind in the same frame is the same animal found twice.
pub const SAME_INSIDE: f64 = 0.6;
/// Clips start this long before the first sighting and end this long after the last.
pub const CLIP_PAD_S: f64 = 1.0;

fn iou(a: [f64; 4], b: [f64; 4]) -> f64 {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    let i = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
    let u = a[2] * a[3] + b[2] * b[3] - i;
    if u <= 0.0 { 0.0 } else { i / u }
}

/// The share of the smaller box that lies inside the other.
fn inside(a: [f64; 4], b: [f64; 4]) -> f64 {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    let i = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
    let small = (a[2] * a[3]).min(b[2] * b[3]);
    if small <= 0.0 { 0.0 } else { i / small }
}

fn jump(a: [f64; 4], b: [f64; 4]) -> f64 { ((a[0] + a[2] / 2.0 - b[0] - b[2] / 2.0).powi(2) + (a[1] + a[3] / 2.0 - b[1] - b[3] / 2.0).powi(2)).sqrt() }

/// Joins boxes into tracks. Faint and empty boxes are dropped, and so is a box mostly inside a more confident one of the
/// same kind in the same frame. Frame by frame, the most confident boxes pick first: each joins the open track of the same
/// category that fits it best (overlap first, then distance), or starts a new one. A track not seen for `MAX_GAP_S` closes.
/// Tracks whose best box stays below `MIN_TRACK_CONF` are dropped.
pub fn track(mut boxes: Vec<Seen>) -> Vec<Track> {
    boxes.retain(|b| b.conf >= MIN_BOX_CONF && b.bbox[2] > 0.005 && b.bbox[3] > 0.005);
    boxes.sort_by(|a, b| a.t.total_cmp(&b.t).then(b.conf.total_cmp(&a.conf)));
    let mut keep = vec![true; boxes.len()];
    for i in 0..boxes.len() {
        for j in 0..i {
            if keep[j] && boxes[j].t == boxes[i].t && boxes[j].category == boxes[i].category && inside(boxes[j].bbox, boxes[i].bbox) >= SAME_INSIDE { keep[i] = false; break; }
        }
    }
    let mut k = keep.into_iter();
    boxes.retain(|_| k.next().unwrap());
    let mut tracks: Vec<Track> = Vec::new();
    let mut i = 0;
    while i < boxes.len() {
        let t = boxes[i].t;
        let mut j = i;
        while j < boxes.len() && boxes[j].t == t { j += 1; }
        let mut taken: Vec<usize> = Vec::new();
        for b in &boxes[i..j] {
            let best = tracks.iter().enumerate()
                .filter(|(k, tr)| !taken.contains(k) && { let last = tr.seen.last().unwrap(); last.category == b.category && t - last.t <= MAX_GAP_S + 1e-9 && last.t < t })
                .map(|(k, tr)| { let last = tr.seen.last().unwrap(); (k, iou(last.bbox, b.bbox), jump(last.bbox, b.bbox)) })
                .filter(|&(_, o, d)| o >= MIN_IOU || d <= MAX_JUMP)
                .max_by(|x, y| x.1.total_cmp(&y.1).then(y.2.total_cmp(&x.2)));
            match best {
                Some((k, _, _)) => { tracks[k].seen.push(b.clone()); taken.push(k); }
                None => { tracks.push(Track { seen: vec![b.clone()] }); taken.push(tracks.len() - 1); }
            }
        }
        i = j;
    }
    tracks.retain(|t| t.seen.iter().any(|s| s.conf >= MIN_TRACK_CONF));
    tracks
}

impl Track {
    pub fn start(&self) -> f64 { self.seen.first().unwrap().t }
    pub fn last(&self) -> f64 { self.seen.last().unwrap().t }

    /// The name the frames agree on: the label with the highest summed score among the frames SpeciesNet named, and a
    /// score that is that sum over every named frame (so a sighting whose frames disagree scores lower). None when no
    /// frame was named (boxes too faint to classify).
    pub fn verdict(&self) -> Option<(String, f64)> {
        let named: Vec<&Seen> = self.seen.iter().filter(|s| s.label.is_some()).collect();
        if named.is_empty() { return None; }
        let mut sums: HashMap<&str, f64> = HashMap::new();
        for s in &named { *sums.entry(s.label.as_deref().unwrap()).or_default() += s.score; }
        let (label, sum) = sums.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(a.0)))?;
        Some((label.to_string(), sum / named.len() as f64))
    }

    /// The frame to keep: the sharpest of the boxes nearly as confident as the most confident one (a sharp box that is
    /// mostly out of the frame is not a good picture). With three frames or more, the first and last are left out: the
    /// animal is often coming or going there, and cameras print their title over the first second.
    pub fn best(&self) -> &Seen {
        let inner: &[Seen] = if self.seen.len() >= 3 { &self.seen[1..self.seen.len() - 1] } else { &self.seen };
        let top = inner.iter().map(|s| s.conf).fold(0.0, f64::max);
        let ok: Vec<&Seen> = inner.iter().filter(|s| s.conf >= top * 0.8).collect();
        ok.into_iter().max_by(|a, b| a.sharp.total_cmp(&b.sharp).then(b.t.total_cmp(&a.t))).unwrap()
    }
}

/// The folder next to a project where its sighting stills and clips go ("Field season.wlnw" -> "Field season media").
pub fn media_dir(project: &Path) -> PathBuf {
    let stem = project.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "project".into());
    project.with_file_name(format!("{stem} media")).join("videos")
}

/// Cuts a clip from `from` to `to` seconds as H.264 MP4 (at most 960 pixels wide, sound kept) that the app can play,
/// whatever the camera wrote.
pub fn cut_clip(video: &Path, from: f64, to: f64, out: &Path) -> Result<()> {
    let tmp = out.with_extension("part.mp4");
    let mut c = crate::video::command()?;
    c.args(["-v", "error", "-y", "-ss"]).arg(format!("{from:.3}")).arg("-i").arg(video).arg("-t").arg(format!("{:.3}", (to - from).max(0.5)))
        .args(["-map", "0:v:0", "-map", "0:a:0?", "-vf", "scale='min(960,iw)':-2", "-c:v", "libx264", "-preset", "veryfast", "-crf", "26", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart"]).arg(&tmp).stdout(Stdio::null()).stderr(Stdio::null());
    if !c.status()?.success() || !tmp.exists() { std::fs::remove_file(&tmp).ok(); anyhow::bail!("could not cut a clip from {}", video.display()); }
    std::fs::rename(&tmp, out)?;
    Ok(())
}

/// Removes a video's sightings (rows and their files).
pub fn clear(p: &Project, tx: &Transaction, video_id: i64) -> Result<()> {
    let files: Vec<(String, Option<String>)> = tx.prepare("SELECT path, clip FROM images WHERE video_id = ?1")?.query_map([video_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let _ = p;
    for (still, clip) in files { std::fs::remove_file(still).ok(); if let Some(c) = clip { std::fs::remove_file(c).ok(); } }
    tx.execute("DELETE FROM images WHERE video_id = ?1", [video_id])?;
    Ok(())
}

/// Builds the sightings of one video from its boxes for `model`: still, clip, and an image row with its one box.
/// Returns how many. Called by frames.rs inside the video's transaction.
pub fn build(p: &Project, tx: &Transaction, video_id: i64, model: &str) -> Result<i64> {
    clear(p, tx, video_id)?;
    let (path, dep, taken, per_s): (String, i64, Option<String>, Option<f64>) = tx.query_row(
        "SELECT v.path, v.deployment_id, v.taken_local, (SELECT per_s FROM video_models WHERE video_id = v.id AND model = ?2) FROM videos v WHERE v.id = ?1",
        params![video_id, model], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    let step = 1.0 / per_s.unwrap_or_else(|| crate::frames::per_s(p));
    let boxes: Vec<Seen> = tx.prepare("SELECT t, category, conf, x, y, w, h, kind, label, score, source, top, sharp FROM video_boxes WHERE video_id = ?1 AND model = ?2")?
        .query_map(params![video_id, model], |r| Ok(Seen { t: r.get(0)?, category: r.get(1)?, conf: r.get(2)?, bbox: [r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?],
            kind: r.get(7)?, label: r.get(8)?, score: r.get(9)?, source: r.get(10)?, top: r.get::<_, Option<String>>(11)?.unwrap_or_else(|| "[]".into()), sharp: r.get(12)? }))?
        .collect::<Result<_, _>>()?;
    let tracks = track(boxes);
    if tracks.is_empty() { return Ok(0); }
    let video = Path::new(&path);
    let dir = media_dir(&p.path);
    std::fs::create_dir_all(&dir)?;
    let duration: Option<f64> = tx.query_row("SELECT duration_s FROM videos WHERE id = ?1", [video_id], |r| r.get(0))?;
    let start_time = taken.as_deref().and_then(|t| chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S").ok());
    let mut species: HashMap<String, i64> = HashMap::new();
    let mut n = 0;
    for (k, tr) in tracks.iter().enumerate() {
        let best = tr.best();
        let (start, end) = (tr.start(), (tr.last() + step).min(duration.unwrap_or(f64::MAX)));
        let stem = format!("v{video_id}_{:02}_{}", k + 1, video.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
        let still = dir.join(format!("{stem}.jpg"));
        crate::frames::still(video, best.t, &still)?;
        let clip = dir.join(format!("{stem}.mp4"));
        // A sighting without its clip is still a sighting: the still and times are what count.
        let clip = cut_clip(video, (start - CLIP_PAD_S).max(0.0), end + CLIP_PAD_S, &clip).ok().map(|_| clip);
        let (w, h) = image::image_dimensions(&still).map(|(w, h)| (Some(w as i64), Some(h as i64))).unwrap_or((None, None));
        let at = start_time.map(|t| (t + chrono::Duration::milliseconds((start * 1000.0) as i64)).format("%Y-%m-%d %H:%M:%S").to_string());
        tx.execute("INSERT INTO images (deployment_id, path, taken_local, time_source, bytes, width, height, video_id, start_s, end_s, clip)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![dep, still.to_string_lossy(), at, at.as_ref().map(|_| "video"), std::fs::metadata(&still).map(|m| m.len() as i64).ok(), w, h,
                video_id, start, end, clip.as_ref().map(|c| c.to_string_lossy().to_string())])?;
        let image_id = tx.last_insert_rowid();
        let (label, score) = tr.verdict().map(|(l, s)| (Some(l), s)).unwrap_or((None, best.conf));
        let kind = match &label { Some(l) => camtrap::taxon(l).kind.to_string(), None => best.kind.clone() };
        let sp = match (&label, kind.as_str()) {
            (Some(l), "animal") => { let t = camtrap::taxon(l); Some(match species.get(&t.scientific) { Some(&s) => s, None => { let s = p.species_id(&t.scientific, &t.common)?; species.insert(t.scientific.clone(), s); s } }) }
            _ => None,
        };
        // The best frame's own top classes; the name and score are the sighting's.
        let b = best.bbox;
        tx.execute("INSERT INTO image_boxes (image_id, model, category, conf, x, y, w, h, kind, species, label, score, source, top) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![image_id, model, best.category, best.conf, b[0], b[1], b[2], b[3], kind, sp, label, score, best.source, best.top])?;
        tx.execute("INSERT INTO image_models (image_id, model, status, error, processed_at) VALUES (?1, ?2, 'done', NULL, datetime('now'))", params![image_id, model])?;
        n += 1;
    }
    Ok(n)
}

/// Rebuilds every processed video's sightings from its stored boxes (after the tracking rules change). Returns how many.
pub fn rebuild_all(p: &Project) -> Result<i64> {
    let done: Vec<(i64, String)> = p.db.prepare("SELECT video_id, model FROM video_models WHERE status = 'done'")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let mut n = 0;
    for (v, m) in done { let tx = p.db.unchecked_transaction()?; n += build(p, &tx, v, &m)?; tx.commit()?; }
    Ok(n)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Listed { pub video: String, pub start_s: f64, pub end_s: f64, pub kind: String, pub name: String, pub score: f64 }

/// Every sighting, by video and time.
pub fn list(p: &Project) -> Result<Vec<Listed>> {
    Ok(p.db.prepare("SELECT v.path, i.start_s, i.end_s, b.kind, COALESCE(s.common, s.scientific, b.label, ''), b.score FROM images i JOIN videos v ON v.id = i.video_id
        JOIN image_boxes b ON b.image_id = i.id LEFT JOIN species s ON s.id = b.species ORDER BY v.path, i.start_s")?
        .query_map([], |r| Ok(Listed { video: r.get(0)?, start_s: r.get(1)?, end_s: r.get(2)?, kind: r.get(3)?, name: r.get(4)?, score: r.get(5)? }))?.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(t: f64, x: f64, label: &str, score: f64, sharp: f64) -> Seen {
        Seen { t, category: "animal".into(), conf: 0.9, bbox: [x, 0.4, 0.2, 0.2], kind: "animal".into(), label: (!label.is_empty()).then(|| label.to_string()),
            score, source: None, top: "[]".into(), sharp }
    }

    #[test]
    fn one_animal_is_one_sighting() {
        // A fox crossing left to right, one frame a second, missed once by the detector.
        let fox = "x;mammalia;carnivora;canidae;urocyon;cinereoargenteus;grey fox";
        let mut b: Vec<Seen> = [0.0, 1.0, 2.0, 4.0, 5.0].iter().enumerate().map(|(i, &t)| seen(t, 0.05 + 0.15 * i as f64, fox, 0.9, 10.0 + i as f64)).collect();
        b[2].label = Some("x;mammalia;carnivora;canidae;;;canine family".into());
        let tr = track(b.clone());
        assert_eq!(tr.len(), 1);
        assert_eq!((tr[0].start(), tr[0].last()), (0.0, 5.0));
        let (label, score) = tr[0].verdict().unwrap();
        assert_eq!(label, fox);
        assert!((score - 0.72).abs() < 1e-9); // 4 x 0.9 of 5 named frames
        // Sharpest of the confident frames, the first and last left out.
        assert_eq!(tr[0].best().t, 4.0);
        let mut blurred_middle = b.clone();
        blurred_middle[3].conf = 0.5; // the sharpest inner frame, but much less sure than the others
        assert_eq!(track(blurred_middle)[0].best().t, 2.0);
        // Gone longer than MAX_GAP_S: a second sighting.
        let mut late = b.clone();
        late.push(seen(9.5, 0.7, fox, 0.9, 1.0));
        assert_eq!(track(late).len(), 2);
    }

    #[test]
    fn two_animals_side_by_side_stay_apart() {
        let deer = "y;mammalia;cetartiodactyla;cervidae;odocoileus;virginianus;white-tailed deer";
        let b: Vec<Seen> = (0..4).flat_map(|t| [seen(t as f64, 0.05, deer, 0.8, 5.0), seen(t as f64, 0.7, deer, 0.8, 5.0)]).collect();
        let tr = track(b);
        assert_eq!(tr.len(), 2);
        assert!(tr.iter().all(|t| t.seen.len() == 4));
        // A person and an animal in the same place are not one track.
        let mut person = seen(1.0, 0.05, "", 0.9, 1.0);
        person.category = "human".into();
        let tr = track(vec![seen(0.0, 0.05, deer, 0.8, 5.0), person, seen(2.0, 0.05, deer, 0.8, 5.0)]);
        assert_eq!(tr.len(), 2);
        // Unnamed boxes only: no verdict.
        assert!(track(vec![seen(0.0, 0.1, "", 0.05, 1.0)])[0].verdict().is_none());
        // Faint boxes and empty boxes are no sighting; a box found twice (one inside the other) is one.
        let mut faint = seen(0.0, 0.1, "", 0.05, 1.0);
        faint.conf = 0.15;
        let mut flat = seen(1.0, 0.5, "", 0.5, 1.0);
        flat.bbox[3] = 0.0;
        let mut twice = seen(0.0, 0.6, deer, 0.8, 5.0);
        twice.conf = 0.4;
        twice.bbox = [0.55, 0.35, 0.35, 0.35];
        let tr = track(vec![faint, flat, seen(0.0, 0.6, deer, 0.8, 5.0), twice]);
        assert_eq!(tr.len(), 1);
        assert_eq!(tr[0].seen.len(), 1);
    }

    #[test]
    fn media_sits_next_to_the_project() {
        assert_eq!(media_dir(Path::new("/d/Field season.wlnw")), Path::new("/d/Field season media/videos"));
    }
}
