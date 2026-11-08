//! Camera-trap images: MegaDetector v6 (MIT YOLOv9) finds animals, people and vehicles; Google's SpeciesNet names the
//! animal in each box; SpeciesNet's ensemble rules (with geofencing by country) give one answer per box. Only the two
//! networks are ONNX; everything around them is reproduced here from the Python reference implementations
//! (Pytorch-Wildlife for the detector, the speciesnet package for crops and the ensemble) so the results match
//! (docs/SPIKE.md, spike/speciesnet_check.py).
use crate::models::{Classifier, Label, ModelInfo, Scored};
use anyhow::{bail, Context, Result};
use ort::session::Session;
use ort::value::Tensor;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

pub const ID: &str = "speciesnet-4.0.3a";
/// Folder of the pack inside the models folder.
pub const DIR: &str = "speciesnet";
pub const FILES: &[&str] = &["detector.onnx", "speciesnet_v403a.onnx", "labels.txt", "taxonomy.txt", "geofence.json", "speciesnet.json"];
/// The detector's input side.
pub const DET_SIZE: usize = 640;
/// SpeciesNet's crop side.
pub const CROP: usize = 480;
/// SpeciesNet keeps detector boxes from this confidence (its detector threshold) ...
pub const DET_FLOOR: f32 = 0.01;
/// ... and classifies boxes from this confidence (its default `detection_threshold_for_classification`).
pub const CLASSIFY_FROM: f32 = 0.1;
/// Top classes the ensemble looks at.
const TOP: usize = 5;

pub const BLANK: &str = "f1856211-cfb7-4a5b-9158-c0f72fd09ee6;;;;;;blank";
pub const ANIMAL: &str = "1f689929-883d-4dae-958c-3d57ab5b6c16;;;;;;animal";
pub const HUMAN: &str = "990ae9dd-7a59-4344-afcb-1b7b21368000;mammalia;primates;hominidae;homo;sapiens;human";
pub const VEHICLE: &str = "e2895ed5-780b-48f6-8a11-9e27cb594511;;;;;;vehicle";
pub const UNKNOWN: &str = "f2efdae9-efb8-48fb-8a91-eccf79ab4ffb;no cv result;no cv result;no cv result;no cv result;no cv result;no cv result";

/// Detector classes, as SpeciesNet names them (MegaDetector categories 1, 2, 3).
pub const CLASSES: [&str; 3] = ["animal", "human", "vehicle"];

// ---------------------------------------------------------------------------------------------------------------------
// Images

/// An 8-bit RGB image, rows top to bottom.
#[derive(Clone)]
pub struct Rgb { pub w: usize, pub h: usize, pub px: Vec<u8> }

/// Loads an image the way speciesnet's `load_rgb_image` does: decoded, converted to RGB, EXIF orientation applied.
/// JPEGs are decoded with libjpeg-turbo's decoder (via mozjpeg, its fork), as Pillow does, so the pixels are the same
/// as the reference's; other decoders differ by a few levels (docs/SPIKE.md).
pub fn load(path: &Path) -> Result<Rgb> {
    let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    let im = if bytes.starts_with(&[0xFF, 0xD8]) {
        decode_jpeg(&bytes).with_context(|| format!("{} is not a readable JPEG", path.display()))?
    } else {
        let img = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format()?.decode()
            .with_context(|| format!("{} is not a readable JPEG or PNG", path.display()))?;
        let rgb = img.to_rgb8();
        Rgb { w: rgb.width() as usize, h: rgb.height() as usize, px: rgb.into_raw() }
    };
    Ok(orient(im, crate::photo::orientation(&bytes)))
}

/// libjpeg defaults (islow IDCT, fancy upsampling). Grey becomes RGB by copying; CMYK (Adobe, stored inverted)
/// becomes RGB with Pillow's own conversion. A truncated or corrupt file is an error, as in Pillow.
fn decode_jpeg(bytes: &[u8]) -> Result<Rgb> {
    let r = std::panic::catch_unwind(|| -> std::io::Result<Rgb> {
        let d = mozjpeg::Decompress::new_mem(bytes)?;
        match d.color_space() {
            mozjpeg::ColorSpace::JCS_GRAYSCALE => {
                let mut s = d.grayscale()?;
                let (w, h) = (s.width(), s.height());
                let g: Vec<u8> = s.read_scanlines()?;
                s.finish()?;
                Ok(Rgb { w, h, px: g.iter().flat_map(|&v| [v, v, v]).collect() })
            }
            mozjpeg::ColorSpace::JCS_CMYK | mozjpeg::ColorSpace::JCS_YCCK => {
                let adobe = bytes.windows(5).any(|w| w == b"Adobe");
                let mut s = d.to_colorspace(mozjpeg::ColorSpace::JCS_CMYK)?;
                let (w, h) = (s.width(), s.height());
                let c: Vec<[u8; 4]> = s.read_scanlines()?;
                s.finish()?;
                let mul = |a: u32, b: u32| { let t = a * b + 128; ((t >> 8) + t) >> 8 };
                Ok(Rgb { w, h, px: c.iter().flat_map(|p| {
                    let p = if adobe { [255 - p[0], 255 - p[1], 255 - p[2], 255 - p[3]] } else { *p };
                    let nk = 255 - p[3] as u32;
                    [0, 1, 2].map(|i| (nk as i32 - mul(p[i] as u32, nk) as i32).clamp(0, 255) as u8)
                }).collect() })
            }
            _ => {
                let mut s = d.rgb()?;
                let (w, h) = (s.width(), s.height());
                let px: Vec<[u8; 3]> = s.read_scanlines()?;
                s.finish()?;
                Ok(Rgb { w, h, px: px.into_iter().flatten().collect() })
            }
        }
    });
    match r {
        Ok(Ok(im)) => Ok(im),
        Ok(Err(e)) => bail!("{e}"),
        Err(_) => bail!("the JPEG data is damaged or cut short"),
    }
}

/// EXIF orientation 1 to 8, as PIL's `exif_transpose`.
pub fn orient(im: Rgb, o: u16) -> Rgb {
    if !(2..=8).contains(&o) { return im; }
    let (w, h) = (im.w, im.h);
    let swap = o >= 5;
    let (ow, oh) = if swap { (h, w) } else { (w, h) };
    let mut px = vec![0u8; im.px.len()];
    for y in 0..oh {
        for x in 0..ow {
            // Source pixel for output (x, y).
            let (sx, sy) = match o {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                _ => (w - 1 - y, x), // 8
            };
            let (d, s) = ((y * ow + x) * 3, (sy * w + sx) * 3);
            px[d..d + 3].copy_from_slice(&im.px[s..s + 3]);
        }
    }
    Rgb { w: ow, h: oh, px }
}

/// Pillow's Lanczos resize of an 8-bit RGB image (Resample.c: separable, horizontal pass first, coefficients in
/// fixed point with 22 fractional bits, rounding and clipping to 8 bits after each pass).
pub fn resize_lanczos(im: &Rgb, ow: usize, oh: usize) -> Rgb {
    const PREC: i32 = 22;
    fn sinc(x: f64) -> f64 { if x == 0.0 { 1.0 } else { let x = x * std::f64::consts::PI; x.sin() / x } }
    fn lanczos(x: f64) -> f64 { if (-3.0..3.0).contains(&x) { sinc(x) * sinc(x / 3.0) } else { 0.0 } }
    fn coeffs(in_size: usize, out_size: usize) -> (usize, Vec<(usize, usize)>, Vec<i32>) {
        let scale = in_size as f64 / out_size as f64;
        let filterscale = scale.max(1.0);
        let support = 3.0 * filterscale;
        let ksize = support.ceil() as usize * 2 + 1;
        let mut bounds = Vec::with_capacity(out_size);
        let mut kk = vec![0i32; out_size * ksize];
        let mut k = vec![0f64; ksize];
        for xx in 0..out_size {
            let center = (xx as f64 + 0.5) * scale;
            let ss = 1.0 / filterscale;
            // C's (int) truncates toward zero; a negative start is then clamped to 0 either way.
            let xmin = ((center - support + 0.5) as i64).max(0) as usize;
            let xmax = (((center + support + 0.5) as i64).min(in_size as i64) as usize).saturating_sub(xmin);
            let mut ww = 0.0;
            for (x, kx) in k.iter_mut().enumerate().take(xmax) { let w = lanczos((x as f64 + xmin as f64 - center + 0.5) * ss); *kx = w; ww += w; }
            for x in 0..xmax {
                let v = if ww != 0.0 { k[x] / ww } else { k[x] };
                kk[xx * ksize + x] = if v < 0.0 { (-0.5 + v * (1 << PREC) as f64) as i32 } else { (0.5 + v * (1 << PREC) as f64) as i32 };
            }
            bounds.push((xmin, xmax));
        }
        (ksize, bounds, kk)
    }
    fn clip8(v: i64) -> u8 { if v >= (1i64 << PREC << 8) { 255 } else if v <= 0 { 0 } else { (v >> PREC) as u8 } }
    let need_h = ow != im.w;
    let need_v = oh != im.h;
    let (kh, bh, ch) = coeffs(im.w, ow);
    let (kv, mut bv, cv) = coeffs(im.h, oh);
    let first = bv[0].0;
    let last = bv[oh - 1].0 + bv[oh - 1].1;
    let mut cur = im.clone();
    if need_h {
        for b in bv.iter_mut() { b.0 -= first; }
        let rows = last - first;
        let mut px = vec![0u8; ow * rows * 3];
        for y in 0..rows {
            let src = &im.px[(y + first) * im.w * 3..(y + first + 1) * im.w * 3];
            for (xx, &(xmin, xmax)) in bh.iter().enumerate() {
                let k = &ch[xx * kh..xx * kh + xmax];
                let mut s = [1i64 << (PREC - 1); 3];
                for (x, &kx) in k.iter().enumerate() {
                    let p = (x + xmin) * 3;
                    for c in 0..3 { s[c] += src[p + c] as i64 * kx as i64; }
                }
                for c in 0..3 { px[(y * ow + xx) * 3 + c] = clip8(s[c]); }
            }
        }
        cur = Rgb { w: ow, h: rows, px };
    }
    if need_v {
        let mut px = vec![0u8; ow * oh * 3];
        for (yy, &(ymin, ymax)) in bv.iter().enumerate() {
            let k = &cv[yy * kv..yy * kv + ymax];
            for xx in 0..ow {
                let mut s = [1i64 << (PREC - 1); 3];
                for (y, &ky) in k.iter().enumerate() {
                    let p = ((y + ymin) * ow + xx) * 3;
                    for c in 0..3 { s[c] += cur.px[p + c] as i64 * ky as i64; }
                }
                for c in 0..3 { px[(yy * ow + xx) * 3 + c] = clip8(s[c]); }
            }
        }
        cur = Rgb { w: ow, h: oh, px };
    }
    cur
}

/// How an image was fitted into the detector's square: Pytorch-Wildlife's PadAndResize (Lanczos, grey 114 borders).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Letterbox { pub scale: f32, pub pad_x: usize, pub pad_y: usize }

/// The detector input (1 x 3 x 640 x 640, 0 to 1) and how to map boxes back.
pub fn letterbox(im: &Rgb) -> (Vec<f32>, Letterbox) {
    let s = DET_SIZE as f64;
    let scale = (s / im.w as f64).min(s / im.h as f64);
    let (nw, nh) = ((im.w as f64 * scale) as usize, (im.h as f64 * scale) as usize);
    let r = resize_lanczos(im, nw, nh);
    let (px, py) = ((DET_SIZE - nw) / 2, (DET_SIZE - nh) / 2);
    let plane = DET_SIZE * DET_SIZE;
    let grey = 114.0f32 / 255.0;
    let mut x = vec![grey; 3 * plane];
    for y in 0..nh {
        for xx in 0..nw {
            let p = (y * nw + xx) * 3;
            let o = (y + py) * DET_SIZE + xx + px;
            for c in 0..3 { x[c * plane + o] = r.px[p + c] as f32 / 255.0; }
        }
    }
    (x, Letterbox { scale: scale as f32, pad_x: px, pad_y: py })
}

// ---------------------------------------------------------------------------------------------------------------------
// Detector output

/// One detector box: class 0 animal, 1 human, 2 vehicle; `bbox` = (x, y, width, height) as fractions of the image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Det { pub class: usize, pub conf: f32, pub bbox: [f64; 4] }

/// Intersection over union of two (x1, y1, x2, y2) boxes, as torchvision's NMS computes it.
pub fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let area = |r: &[f32; 4]| (r[2] - r[0]) * (r[3] - r[1]);
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = w * h;
    inter / (area(a) + area(b) - inter)
}

/// YOLOv9's post-processing (bbox_nms): every (anchor, class) above `floor`, NMS per class at `iou_max`, highest first,
/// at most `max` boxes. `scores` is anchors x 3 (already sigmoid), `boxes` anchors x 4 (x1, y1, x2, y2).
pub fn nms(scores: &[f32], boxes: &[f32], floor: f32, iou_max: f32, max: usize) -> Vec<(usize, f32, [f32; 4])> {
    let n = boxes.len() / 4;
    let nc = scores.len() / n.max(1);
    let mut cand: Vec<(usize, f32, [f32; 4])> = Vec::new();
    for a in 0..n {
        for c in 0..nc {
            let s = scores[a * nc + c];
            if s > floor { cand.push((c, s, [boxes[a * 4], boxes[a * 4 + 1], boxes[a * 4 + 2], boxes[a * 4 + 3]])); }
        }
    }
    cand.sort_by(|x, y| y.1.total_cmp(&x.1));
    let mut keep: Vec<(usize, f32, [f32; 4])> = Vec::new();
    let mut dead = vec![false; cand.len()];
    for i in 0..cand.len() {
        if dead[i] { continue; }
        keep.push(cand[i]);
        for j in i + 1..cand.len() {
            if !dead[j] && cand[j].0 == cand[i].0 && iou(&cand[i].2, &cand[j].2) > iou_max { dead[j] = true; }
        }
    }
    keep.truncate(max);
    keep
}

/// Boxes from the letterboxed square back to the image, clipped to it, slivers under 2 px dropped, as SpeciesNet
/// detections (fractions of the image), highest confidence first.
pub fn to_image(found: &[(usize, f32, [f32; 4])], lb: Letterbox, w: usize, h: usize) -> Vec<Det> {
    let (wf, hf) = (w as f64, h as f64);
    let mut out: Vec<Det> = found.iter().filter_map(|&(c, conf, b)| {
        let px = lb.pad_x as f32;
        let py = lb.pad_y as f32;
        let m = [(b[0] - px) / lb.scale, (b[1] - py) / lb.scale, (b[2] - px) / lb.scale, (b[3] - py) / lb.scale];
        let x1 = (m[0] as f64).clamp(0.0, wf);
        let x2 = (m[2] as f64).clamp(0.0, wf);
        let y1 = (m[1] as f64).clamp(0.0, hf);
        let y2 = (m[3] as f64).clamp(0.0, hf);
        if x2 - x1 < 2.0 || y2 - y1 < 2.0 { return None; }
        Some(Det { class: c, conf, bbox: [x1 / wf, y1 / hf, (x2 - x1) / wf, (y2 - y1) / hf] })
    }).collect();
    out.sort_by(|a, b| b.conf.total_cmp(&a.conf));
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// Classifier input

/// SpeciesNet's crop: the box cut from the image (torchvision crop with int() of the scaled box), resized to 480 x 480
/// with bilinear interpolation without antialiasing (align_corners false), converted to 8 bits (x 255.999, truncated)
/// and back to 0 to 1. Returns 480 x 480 x 3 (HWC), as the classifier takes it.
pub fn crop(im: &Rgb, bbox: &[f64; 4]) -> Vec<f32> {
    let (wf, hf) = (im.w as f64, im.h as f64);
    let top = (bbox[1] * hf) as i64;
    let left = (bbox[0] * wf) as i64;
    let ch = ((bbox[3] * hf) as i64).max(1) as usize;
    let cw = ((bbox[2] * wf) as i64).max(1) as usize;
    // torchvision pads with zeros outside the image; boxes are clipped, so this only guards rounding at the edge.
    let at = |y: usize, x: usize, c: usize| -> f32 {
        let (yy, xx) = (top + y as i64, left + x as i64);
        if yy < 0 || xx < 0 || yy >= im.h as i64 || xx >= im.w as i64 { 0.0 } else { im.px[(yy as usize * im.w + xx as usize) * 3 + c] as f32 / 255.0 }
    };
    let idx = |in_size: usize| -> Vec<(usize, usize, f32, f32)> {
        let scale = in_size as f32 / CROP as f32;
        (0..CROP).map(|d| {
            let real = (scale * (d as f32 + 0.5) - 0.5).max(0.0);
            let i0 = (real.floor() as usize).min(in_size - 1);
            let l1 = (real - i0 as f32).clamp(0.0, 1.0);
            let i1 = if i0 < in_size - 1 { i0 + 1 } else { i0 };
            (i0, i1, 1.0 - l1, l1)
        }).collect()
    };
    let (ys, xs) = (idx(ch), idx(cw));
    let mut out = vec![0f32; CROP * CROP * 3];
    for (oy, &(y0, y1, hy0, hy1)) in ys.iter().enumerate() {
        for (ox, &(x0, x1, wx0, wx1)) in xs.iter().enumerate() {
            for c in 0..3 {
                let r0 = at(y0, x0, c) * wx0 + at(y0, x1, c) * wx1;
                let r1 = at(y1, x0, c) * wx0 + at(y1, x1, c) * wx1;
                let v = r0 * hy0 + r1 * hy1;
                let u = (v * 255.999f32) as u8;
                out[(oy * CROP + ox) * 3 + c] = (u as f64 / 255.0) as f32;
            }
        }
    }
    out
}

/// Softmax, then the `k` best (label index, score), best first.
pub fn top_k(logits: &[f32], k: usize) -> Vec<(usize, f32)> {
    let m = logits.iter().cloned().fold(f32::MIN, f32::max);
    let e: Vec<f32> = logits.iter().map(|v| (v - m).exp()).collect();
    let sum: f32 = e.iter().sum();
    let mut idx: Vec<(usize, f32)> = e.iter().enumerate().map(|(i, v)| (i, v / sum)).collect();
    idx.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    idx.truncate(k);
    idx
}

// ---------------------------------------------------------------------------------------------------------------------
// Ensemble (speciesnet ensemble_prediction_combiner.py, geofence_utils.py, taxonomy_utils.py)

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rule {
    #[serde(default)] pub allow: Option<HashMap<String, Vec<String>>>,
    #[serde(default)] pub block: Option<HashMap<String, Vec<String>>>,
}

/// What the ensemble decided for one box: a SpeciesNet label, its score and where it came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Processed { pub label: String, pub score: f64, pub source: String }

pub struct Ensemble { pub taxonomy: HashMap<String, String>, pub geofence: HashMap<String, Rule> }

fn parts(label: &str) -> Vec<&str> { label.split(';').collect() }
fn taxa(label: &str) -> String { label.split(';').skip(1).take(5).collect::<Vec<_>>().join(";") }

impl Ensemble {
    pub fn load(taxonomy: &str, geofence: &str) -> Result<Self> {
        let mut map: HashMap<String, String> = taxonomy.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| (taxa(l), l.to_string())).collect();
        for l in [BLANK, VEHICLE, UNKNOWN] { map.remove(&taxa(l)); }
        for l in [HUMAN, ANIMAL] { map.insert(taxa(l), l.to_string()); }
        Ok(Ensemble { taxonomy: map, geofence: serde_json::from_str(geofence).context("geofence.json")? })
    }

    pub fn ancestor(&self, label: &str, level: &str) -> Option<String> {
        let p = parts(label);
        if p.len() != 7 { return None; }
        let a: Vec<&str> = match level {
            "species" => { if p[5].is_empty() { return None; } p[1..6].to_vec() }
            "genus" => { if p[4].is_empty() { return None; } [&p[1..5], &[""]].concat() }
            "family" => { if p[3].is_empty() { return None; } [&p[1..4], &["", ""]].concat() }
            "order" => { if p[2].is_empty() { return None; } [&p[1..3], &["", "", ""]].concat() }
            "class" => { if p[1].is_empty() { return None; } [&p[1..2], &["", "", "", ""]].concat() }
            "kingdom" => { if p[1].is_empty() && label != ANIMAL { return None; } vec![""; 5] }
            _ => return None,
        };
        self.taxonomy.get(&a.join(";")).cloned()
    }

    pub fn geofenced(&self, label: &str, country: Option<&str>, admin1: Option<&str>) -> bool {
        let Some(country) = country.filter(|c| c.len() == 3) else { return false };
        let admin1 = admin1.filter(|a| a.len() == 2);
        let Some(rule) = self.geofence.get(&taxa(label)) else { return false };
        if let Some(allow) = rule.allow.as_ref().filter(|a| !a.is_empty()) {
            match allow.get(country) {
                None => return true,
                Some(regions) => { if let Some(a) = admin1 { if !regions.is_empty() && !regions.iter().any(|r| r == a) { return true; } } }
            }
        }
        if let Some(block) = rule.block.as_ref().filter(|b| !b.is_empty()) {
            if let Some(regions) = block.get(country) {
                if regions.is_empty() { return true; }
                if let Some(a) = admin1 { if regions.iter().any(|r| r == a) { return true; } }
            }
        }
        false
    }

    /// The first taxonomy level whose summed scores pass `threshold` (geofenced taxa left out).
    pub fn roll_up(&self, top: &[(String, f64)], country: Option<&str>, admin1: Option<&str>, levels: &[&str], threshold: f64) -> Option<Processed> {
        for level in levels {
            // Insertion order, as Python's dict: on a tie the first taxon reached wins.
            let mut acc: Vec<(String, f64)> = Vec::new();
            for (label, score) in top {
                if let Some(r) = self.ancestor(label, level) {
                    match acc.iter_mut().find(|x| x.0 == r) { Some(x) => x.1 += score, None => acc.push((r, *score)) }
                }
            }
            let mut best: Option<(String, f64)> = None;
            for (l, s) in acc {
                if s > best.as_ref().map_or(0.0, |b| b.1) && !self.geofenced(&l, country, admin1) { best = Some((l, s)); }
            }
            if let Some((l, s)) = best { if s > threshold { return Some(Processed { label: l, score: s, source: format!("classifier+rollup_to_{level}") }); } }
        }
        None
    }

    fn geofence_top(&self, top: &[(String, f64)], country: Option<&str>, admin1: Option<&str>) -> Processed {
        if self.geofenced(&top[0].0, country, admin1) {
            match self.roll_up(top, country, admin1, &["family", "order", "class", "kingdom"], top[0].1 - 1e-10) {
                Some(r) => Processed { source: format!("classifier+geofence+{}", &r.source["classifier+".len()..]), ..r },
                None => Processed { label: UNKNOWN.into(), score: top[0].1, source: "classifier+geofence+rollup_failed".into() },
            }
        } else {
            Processed { label: top[0].0.clone(), score: top[0].1, source: "classifier".into() }
        }
    }

    /// One box: the detector's class and confidence, and the classifier's top classes (best first).
    pub fn combine(&self, det_class: usize, det_score: f64, top: &[(String, f64)], country: Option<&str>, admin1: Option<&str>) -> Processed {
        let p = |l: &str, s: f64, src: &str| Some(Processed { label: l.to_string(), score: s, source: src.to_string() });
        let (tc, ts) = (top[0].0.as_str(), top[0].1);
        let mut out: Option<Processed> = None;
        if det_class == 1 {
            if det_score > 0.7 { out = p(HUMAN, det_score, "detector"); }
            else if det_score > 0.2 && (tc == HUMAN || tc == VEHICLE) && ts > 0.5 { out = p(HUMAN, ts, "classifier"); }
        }
        if out.is_none() && det_class == 2 {
            if det_score > 0.2 && tc == HUMAN && ts > 0.5 { out = p(HUMAN, ts, "classifier"); }
            else if det_score > 0.7 { out = p(VEHICLE, det_score, "detector"); }
            else if det_score > 0.2 && tc == VEHICLE && ts > 0.4 { out = p(VEHICLE, ts, "classifier"); }
        }
        if out.is_none() {
            if det_score < 0.2 && tc == BLANK && ts > 0.5 { out = p(BLANK, ts, "classifier"); }
            else if tc == BLANK && ts > 0.99 { out = p(BLANK, ts, "classifier"); }
            else if tc != BLANK && tc != HUMAN && tc != VEHICLE {
                if ts > 0.8 || (ts > 0.65 && det_class == 0 && det_score > 0.2) { out = Some(self.geofence_top(top, country, admin1)); }
            }
        }
        if let Some(o) = out { return o; }
        if let Some(r) = self.roll_up(top, country, admin1, &["genus", "family", "order", "class", "kingdom"], 0.65) { return r; }
        if det_class == 0 && det_score > 0.5 { return Processed { label: ANIMAL.into(), score: det_score, source: "detector".into() }; }
        Processed { label: UNKNOWN.into(), score: ts, source: "classifier".into() }
    }
}

/// What a SpeciesNet label means for Studio.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Taxon { pub kind: &'static str, pub scientific: String, pub common: String, pub rank: &'static str }

/// Kinds: animal (a taxon, or "Animalia" when only the detector is sure), human, vehicle, blank, unknown.
pub fn taxon(label: &str) -> Taxon {
    let t = |kind, s: &str, c: &str, rank| Taxon { kind, scientific: s.to_string(), common: c.to_string(), rank };
    match label {
        HUMAN => return t("human", "Homo sapiens", "human", "species"),
        VEHICLE => return t("vehicle", "", "vehicle", ""),
        BLANK => return t("blank", "", "blank", ""),
        UNKNOWN => return t("unknown", "", "unknown", ""),
        ANIMAL => return t("animal", "Animalia", "animal", "kingdom"),
        _ => {}
    }
    let p = parts(label);
    if p.len() != 7 { return t("unknown", "", "", ""); }
    let cap = |s: &str| { let mut c = s.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() };
    let common = p[6].to_string();
    let (sci, rank) = if !p[4].is_empty() && !p[5].is_empty() { (format!("{} {}", cap(p[4]), p[5]), "species") }
        else if !p[4].is_empty() { (cap(p[4]), "genus") } else if !p[3].is_empty() { (cap(p[3]), "family") }
        else if !p[2].is_empty() { (cap(p[2]), "order") } else if !p[1].is_empty() { (cap(p[1]), "class") } else { ("Animalia".into(), "kingdom") };
    if sci.eq_ignore_ascii_case("homo sapiens") { return t("human", "Homo sapiens", "human", "species"); }
    Taxon { kind: "animal", scientific: sci, common, rank }
}

// ---------------------------------------------------------------------------------------------------------------------
// The model pack

#[derive(Debug, Clone, Deserialize)]
struct PackConfig { speciesnet_version: String, detector: String, nms_iou: f32, max_boxes: usize }

/// One box with what SpeciesNet made of it (boxes below `CLASSIFY_FROM` are not classified: `processed` is None).
#[derive(Debug, Clone, Serialize)]
pub struct Found { pub det: Det, pub processed: Option<Processed>, pub top: Vec<(String, f64)> }

pub struct SpeciesNet {
    info: ModelInfo, labels: Vec<Label>, raw_labels: Vec<String>, cfg: PackConfig,
    det: Mutex<Session>, clf: Mutex<Session>, pub ensemble: Ensemble,
}

impl SpeciesNet {
    pub fn load(dir: &Path) -> Result<Self> {
        let cfg: PackConfig = serde_json::from_str(&std::fs::read_to_string(dir.join("speciesnet.json")).context("speciesnet.json")?)?;
        let raw_labels: Vec<String> = std::fs::read_to_string(dir.join("labels.txt")).context("labels.txt")?.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        // Named animal taxa only (blank, vehicle, people and "animal" are not species).
        let labels = raw_labels.iter().map(|l| taxon(l)).filter(|t| t.kind == "animal" && t.rank != "kingdom").map(|t| Label { scientific: t.scientific, common: t.common }).collect();
        let ensemble = Ensemble::load(&std::fs::read_to_string(dir.join("taxonomy.txt")).context("taxonomy.txt")?,
            &std::fs::read_to_string(dir.join("geofence.json")).context("geofence.json")?)?;
        let threads = crate::birdnet::threads();
        Ok(SpeciesNet {
            info: ModelInfo {
                id: ID.into(), name: "SpeciesNet".into(), version: format!("{} with {}", cfg.speciesnet_version, cfg.detector), licence: "Apache-2.0".into(),
                licence_url: "https://www.apache.org/licenses/LICENSE-2.0".into(),
                citation: "Gadot et al. (2024), To crop or not to crop: Comparing whole-image and cropped classification on a large dataset of camera trap images, IET Computer Vision, doi:10.1049/cvi2.12318; detector: MegaDetector v6 (Microsoft Pytorch-Wildlife, MIT)".into(),
                sample_rate: 0, window: 0, min_len: 0, embeds: false, location_filter: true, speed_x: 0.0,
            },
            labels, raw_labels, cfg,
            det: Mutex::new(crate::birdnet::session(dir.join("detector.onnx"), threads)?),
            clf: Mutex::new(crate::birdnet::session(dir.join("speciesnet_v403a.onnx"), threads)?),
            ensemble,
        })
    }

    /// The detector on one image: boxes at or above SpeciesNet's detector threshold, highest first.
    pub fn detect(&self, im: &Rgb) -> Result<Vec<Det>> {
        let (x, lb) = letterbox(im);
        let t = Tensor::from_array(([1usize, 3, DET_SIZE, DET_SIZE], x))?;
        let mut s = self.det.lock().unwrap();
        let out = s.run(ort::inputs!["image" => t])?;
        let (_, scores) = out["scores"].try_extract_tensor::<f32>()?;
        let (_, boxes) = out["boxes"].try_extract_tensor::<f32>()?;
        let found = nms(scores, boxes, DET_FLOOR, self.cfg.nms_iou, self.cfg.max_boxes);
        Ok(to_image(&found, lb, im.w, im.h))
    }

    /// The classifier's top classes for each box (batches of 8, as speciesnet).
    pub fn classify(&self, im: &Rgb, boxes: &[[f64; 4]]) -> Result<Vec<Vec<(String, f64)>>> {
        let mut out = Vec::with_capacity(boxes.len());
        for chunk in boxes.chunks(8) {
            let input: Vec<f32> = chunk.iter().flat_map(|b| crop(im, b)).collect();
            let t = Tensor::from_array(([chunk.len(), CROP, CROP, 3], input))?;
            let mut s = self.clf.lock().unwrap();
            let r = s.run(ort::inputs!["crop" => t])?;
            let (_, logits) = r["logits"].try_extract_tensor::<f32>()?;
            let n = self.raw_labels.len();
            for b in 0..chunk.len() {
                out.push(top_k(&logits[b * n..(b + 1) * n], TOP).into_iter().map(|(i, s)| (self.raw_labels[i].clone(), s as f64)).collect());
            }
        }
        Ok(out)
    }

    /// The whole SpeciesNet pipeline on one image. `country` is ISO 3166-1 alpha-3 (geofencing off without it),
    /// `admin1` a US state code.
    pub fn process(&self, im: &Rgb, country: Option<&str>, admin1: Option<&str>) -> Result<Vec<Found>> {
        let dets = self.detect(im)?;
        let pick: Vec<usize> = (0..dets.len()).filter(|&i| dets[i].conf >= CLASSIFY_FROM).collect();
        let tops = self.classify(im, &pick.iter().map(|&i| dets[i].bbox).collect::<Vec<_>>())?;
        let mut out: Vec<Found> = dets.into_iter().map(|d| Found { det: d, processed: None, top: vec![] }).collect();
        for (&i, top) in pick.iter().zip(tops) {
            let p = self.ensemble.combine(out[i].det.class, out[i].det.conf as f64, &top, country, admin1);
            out[i].processed = Some(p);
            out[i].top = top;
        }
        Ok(out)
    }
}

impl Classifier for SpeciesNet {
    fn info(&self) -> &ModelInfo { &self.info }
    fn labels(&self) -> &[Label] { &self.labels }
    fn predict(&self, _windows: &[Vec<f32>]) -> Result<Vec<Scored>> { bail!("SpeciesNet reads images, not sound") }
    fn images(&self) -> Option<&SpeciesNet> { Some(self) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ens() -> Ensemble {
        let tax = "a;mammalia;carnivora;felidae;panthera;leo;lion\nb;mammalia;carnivora;felidae;panthera;;panthera species\nc;mammalia;carnivora;felidae;;;cat family\n\
                   d;mammalia;carnivora;;;;carnivorous mammal\ne;mammalia;;;;;mammal\nf;mammalia;carnivora;felidae;panthera;pardus;leopard\n\
                   g;mammalia;carnivora;canidae;canis;latrans;coyote\nh;mammalia;carnivora;canidae;;;dog family";
        let geo = r#"{"mammalia;carnivora;felidae;panthera;leo": {"allow": {"KEN": [], "TZA": []}},
                      "mammalia;carnivora;canidae;canis;latrans": {"block": {"GBR": []}},
                      "mammalia;carnivora;felidae;panthera;pardus": {"allow": {"USA": ["CA"]}}}"#;
        Ensemble::load(tax, geo).unwrap()
    }
    const LION: &str = "a;mammalia;carnivora;felidae;panthera;leo;lion";
    const LEOPARD: &str = "f;mammalia;carnivora;felidae;panthera;pardus;leopard";
    const COYOTE: &str = "g;mammalia;carnivora;canidae;canis;latrans;coyote";
    fn top(v: &[(&str, f64)]) -> Vec<(String, f64)> { v.iter().map(|(l, s)| (l.to_string(), *s)).collect() }

    #[test]
    fn nms_per_class_and_order() {
        // Anchors: two overlapping animal boxes, one person box at the same place, one far animal box.
        let scores = [0.9, 0.0, 0.0, 0.8, 0.0, 0.0, 0.0, 0.7, 0.0, 0.3, 0.0, 0.0, 0.005, 0.0, 0.0];
        let boxes = [0., 0., 10., 10., 1., 1., 11., 11., 0., 0., 10., 10., 50., 50., 60., 60., 70., 70., 80., 80.];
        let k = nms(&scores, &boxes, 0.01, 0.5, 300);
        assert_eq!(k.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(), vec![(0, 0.9), (1, 0.7), (0, 0.3)]);
        // IoU of (0,0,10,10) and (1,1,11,11) = 81 / 119 > 0.5: suppressed. Exactly at the limit is kept (strictly greater only).
        assert!((iou(&[0., 0., 10., 10.], &[1., 1., 11., 11.]) - 81.0 / 119.0).abs() < 1e-6);
        assert_eq!(nms(&scores, &boxes, 0.01, 0.99, 1).len(), 1);
    }

    #[test]
    fn boxes_back_to_the_image() {
        // A 2048 x 1536 image: scale 0.3125, 640 x 480, padded 80 above and below.
        let lb = Letterbox { scale: 0.3125, pad_x: 0, pad_y: 80 };
        let d = to_image(&[(0, 0.5, [64.0, 80.0, 320.0, 560.0]), (1, 0.9, [-10.0, 100.0, 0.5, 130.0]), (2, 0.4, [600.0, 500.0, 700.0, 600.0])], lb, 2048, 1536);
        assert_eq!(d.len(), 2); // the person box is under 2 px wide once clipped
        assert_eq!(d[0].conf, 0.5);
        assert!((d[0].bbox[0] - 0.1).abs() < 1e-6 && d[0].bbox[1].abs() < 1e-6 && (d[0].bbox[2] - 0.4).abs() < 1e-6 && (d[0].bbox[3] - 1.0).abs() < 1e-6);
        assert!((d[1].bbox[0] + d[1].bbox[2] - 1.0).abs() < 1e-6 && (d[1].bbox[1] + d[1].bbox[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn lanczos_identity_and_flat() {
        let im = Rgb { w: 7, h: 5, px: (0..105).map(|v| (v * 2) as u8).collect() };
        assert_eq!(resize_lanczos(&im, 7, 5).px, im.px);
        let flat = Rgb { w: 100, h: 60, px: vec![77; 100 * 60 * 3] };
        let r = resize_lanczos(&flat, 33, 20);
        assert!(r.px.iter().all(|&v| v == 77));
        let (x, lb) = letterbox(&flat);
        assert_eq!(lb, Letterbox { scale: 6.4, pad_x: 0, pad_y: 128 });
        assert!((x[0] - 114.0 / 255.0).abs() < 1e-7 && (x[200 * 640 + 5] - 77.0 / 255.0).abs() < 1e-7);
    }

    #[test]
    fn orientation() {
        // 2 x 1 image: red, green. Orientation 6 (rotate 90 clockwise to view) gives 1 x 2: red on top.
        let im = Rgb { w: 2, h: 1, px: vec![255, 0, 0, 0, 255, 0] };
        let r = orient(im.clone(), 6);
        assert_eq!((r.w, r.h, r.px.clone()), (1, 2, vec![255, 0, 0, 0, 255, 0]));
        let r = orient(im.clone(), 8);
        assert_eq!((r.w, r.h, r.px), (1, 2, vec![0, 255, 0, 255, 0, 0]));
        assert_eq!(orient(im.clone(), 3).px, vec![0, 255, 0, 255, 0, 0]);
    }

    #[test]
    fn ensemble_rules() {
        let e = ens();
        // Confident classifier, no country: the species.
        let r = e.combine(0, 0.9, &top(&[(LION, 0.95), (LEOPARD, 0.03)]), None, None);
        assert_eq!((r.label.as_str(), r.source.as_str()), (LION, "classifier"));
        // Lions outside their allowed countries are geofenced: rolled up to the first level holding the top score.
        let r = e.combine(0, 0.9, &top(&[(LION, 0.95), (LEOPARD, 0.03)]), Some("USA"), None);
        assert_eq!(r.label, "c;mammalia;carnivora;felidae;;;cat family");
        assert_eq!(r.source, "classifier+geofence+rollup_to_family");
        assert!((r.score - 0.98).abs() < 1e-12);
        assert!(!e.geofenced(LION, Some("KEN"), None));
        // Block lists, and allow lists with regions.
        assert!(e.geofenced(COYOTE, Some("GBR"), None) && !e.geofenced(COYOTE, Some("USA"), None));
        assert!(e.geofenced(LEOPARD, Some("USA"), Some("NY")) && !e.geofenced(LEOPARD, Some("USA"), Some("CA")) && !e.geofenced(LEOPARD, Some("USA"), None));
        // People and vehicles from a confident detector.
        assert_eq!(e.combine(1, 0.8, &top(&[(LION, 0.9)]), None, None).label, HUMAN);
        assert_eq!(e.combine(2, 0.75, &top(&[(LION, 0.9)]), None, None).label, VEHICLE);
        let r = e.combine(2, 0.3, &top(&[(HUMAN, 0.6)]), None, None);
        assert_eq!((r.label.as_str(), r.source.as_str(), r.score), (HUMAN, "classifier", 0.6));
        // Blank only when the detector is unsure, or the classifier is almost certain.
        assert_eq!(e.combine(0, 0.1, &top(&[(BLANK, 0.6)]), None, None).label, BLANK);
        assert_eq!(e.combine(0, 0.6, &top(&[(BLANK, 0.995)]), None, None).label, BLANK);
        assert_eq!(e.combine(0, 0.6, &top(&[(BLANK, 0.6), (LION, 0.3)]), None, None).label, ANIMAL);
        // Unsure species: rolled up to the genus when the scores add up past 0.65.
        let r = e.combine(0, 0.15, &top(&[(LION, 0.4), (LEOPARD, 0.35), (COYOTE, 0.2)]), None, None);
        assert_eq!((r.label.as_str(), r.source.as_str()), ("b;mammalia;carnivora;felidae;panthera;;panthera species", "classifier+rollup_to_genus"));
        // 0.65 to 0.8 needs a confident animal box.
        assert_eq!(e.combine(0, 0.3, &top(&[(LION, 0.7)]), None, None).label, LION);
        let r = e.combine(0, 0.1, &top(&[(LION, 0.7)]), None, None);
        assert_eq!(r.source, "classifier+rollup_to_genus");
        // Nothing adds up and the detector is unsure: unknown with the classifier's score.
        let r = e.combine(0, 0.15, &top(&[(LION, 0.3), (COYOTE, 0.3)]), None, None);
        assert_eq!((r.label.as_str(), r.score), (UNKNOWN, 0.3));
        // ... unless the detector is sure it is an animal.
        assert_eq!(e.combine(0, 0.6, &top(&[(LION, 0.3), (COYOTE, 0.3)]), None, None).label, ANIMAL);
    }

    #[test]
    fn taxa_names() {
        assert_eq!(taxon(LION), Taxon { kind: "animal", scientific: "Panthera leo".into(), common: "lion".into(), rank: "species" });
        assert_eq!(taxon("c;mammalia;carnivora;felidae;;;cat family").scientific, "Felidae");
        assert_eq!(taxon("c;mammalia;carnivora;felidae;;;cat family").rank, "family");
        assert_eq!(taxon(HUMAN).kind, "human");
        assert_eq!(taxon(VEHICLE).kind, "vehicle");
        assert_eq!(taxon(BLANK).kind, "blank");
        assert_eq!(taxon(ANIMAL).scientific, "Animalia");
    }

    #[test]
    fn top_k_softmax() {
        let t = top_k(&[1.0, 3.0, 2.0], 2);
        assert_eq!(t.iter().map(|x| x.0).collect::<Vec<_>>(), vec![1, 2]);
        let e = [1f32.exp(), 3f32.exp(), 2f32.exp()];
        assert!((t[0].1 - e[1] / e.iter().sum::<f32>()).abs() < 1e-6);
    }
}
