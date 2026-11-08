//! Compares Studio's camera-trap pipeline with the Python reference written by spike/speciesnet_check.py.
//!   cargo run --release -p wlnw-engine --example camtrap_check -- models/speciesnet IMAGES reference.json [COUNTRY] [TENSOR_DIR]
use std::path::Path;
use std::time::Instant;
use wlnw_engine::camtrap::{self, SpeciesNet};

fn npy(path: &Path) -> Option<(Vec<usize>, Vec<u8>)> {
    let b = std::fs::read(path).ok()?;
    let hl = u16::from_le_bytes([b[8], b[9]]) as usize;
    let header = String::from_utf8_lossy(&b[10..10 + hl]).to_string();
    let shape: Vec<usize> = header.split("'shape': (").nth(1)?.split(')').next()?.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    Some((shape, b[10 + hl..].to_vec()))
}

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (pack, images, reference) = (Path::new(&a[1]), Path::new(&a[2]), Path::new(&a[3]));
    let country = a.get(4).map(String::as_str).filter(|s| !s.is_empty() && *s != "-");
    let r: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(reference)?)?;
    let model = SpeciesNet::load(pack)?;
    let list = r["images"].as_array().unwrap();

    if let Some(dir) = a.get(5).map(Path::new) {
        for n in ["african_elephants", "african_elephants_cmyk", "african_elephants_bw", "ocelot"] {
            let Some((shape, raw)) = npy(&dir.join(format!("dec_{n}.npy"))) else { continue };
            let im = camtrap::load(&images.join(format!("{n}.jpg")))?;
            let diff = im.px.iter().zip(&raw).map(|(&x, &y)| (x as i32 - y as i32).abs()).max().unwrap_or(0);
            let off = im.px.iter().zip(&raw).filter(|(x, y)| x != y).count();
            println!("decoded {n} vs PIL: shape {:?} vs {}x{}, max diff {diff} levels, {off} of {} values differ", shape, im.h, im.w, raw.len());
        }
        let first = images.join(list[0]["file"].as_str().unwrap());
        let im = camtrap::load(&first)?;
        if let Some((_, raw)) = npy(&dir.join("det_input.npy")) {
            let want: Vec<f32> = raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            let (got, _) = camtrap::letterbox(&im);
            let diff = got.iter().zip(&want).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
            let off = got.iter().zip(&want).filter(|(x, y)| (*x - *y).abs() > 1e-6).count();
            println!("detector input vs Pytorch-Wildlife: max diff {:.6} ({:.1} levels), {} of {} values differ", diff, diff * 255.0, off, want.len());
            // The same from PIL's decoded pixels: isolates the resize from JPEG decoding.
            if let Some((shape, raw)) = npy(&dir.join("dec_african_elephants.npy")) {
                let pil = camtrap::Rgb { w: shape[1], h: shape[0], px: raw };
                let (got, _) = camtrap::letterbox(&pil);
                let diff = got.iter().zip(&want).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
                let off = got.iter().zip(&want).filter(|(x, y)| (*x - *y).abs() > 1e-6).count();
                println!("  from PIL's pixels (resize only): max diff {:.1} levels, {} values differ", diff * 255.0, off);
            }
        }
        if let Some((_, raw)) = npy(&dir.join("crop0.npy")) {
            let dets = model.detect(&im)?;
            let got = camtrap::crop(&im, &dets[0].bbox);
            let diff = got.iter().zip(&raw).map(|(x, &y)| (x * 255.0 - y as f32).abs()).fold(0f32, f32::max);
            let off = got.iter().zip(&raw).filter(|(x, &y)| (*x * 255.0 - y as f32).abs() > 0.5).count();
            println!("first crop vs speciesnet: max diff {:.1} levels, {} of {} values differ", diff, off, raw.len());
        }
    }

    let (mut min_iou, mut max_conf, mut max_score, mut boxes, mut same, mut classified, mut missing, mut extra) = (1f64, 0f64, 0f64, 0, 0, 0, 0, 0);
    let mut secs = 0.0;
    for img in list {
        let file = img["file"].as_str().unwrap();
        let t = Instant::now();
        let im = camtrap::load(&images.join(file))?;
        let got = model.process(&im, country, None)?;
        secs += t.elapsed().as_secs_f64();
        let want: Vec<&serde_json::Value> = img["detections"].as_array().unwrap().iter().filter(|d| d["conf"].as_f64().unwrap() >= camtrap::CLASSIFY_FROM as f64).collect();
        let got: Vec<&camtrap::Found> = got.iter().filter(|f| f.processed.is_some()).collect();
        let mut used = vec![false; got.len()];
        for w in &want {
            let wb: Vec<f64> = w["bbox"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            let wc = w["category"].as_str().unwrap().parse::<usize>().unwrap() - 1;
            let best = got.iter().enumerate().filter(|(i, g)| !used[*i] && g.det.class == wc)
                .map(|(i, g)| (i, iou(&wb, &g.det.bbox))).max_by(|x, y| x.1.total_cmp(&y.1));
            let Some((i, v)) = best.filter(|b| b.1 > 0.5) else { missing += 1; println!("  {file}: reference box {wb:?} {} not found", w["conf"]); continue };
            used[i] = true;
            boxes += 1;
            min_iou = min_iou.min(v);
            max_conf = max_conf.max((got[i].det.conf as f64 - w["conf"].as_f64().unwrap()).abs());
            let wp = &w["processed_classification"];
            let gp = got[i].processed.as_ref().unwrap();
            classified += 1;
            if wp["class"].as_str() == Some(gp.label.as_str()) { same += 1; } else { println!("  {file}: {} vs {}", wp["class"], gp.label); }
            max_score = max_score.max((wp["score"].as_f64().unwrap() - gp.score).abs());
            let wt = w["raw_classifications"]["classes"][0].as_str().unwrap_or("");
            if wt != got[i].top[0].0 { println!("  {file}: classifier top {wt} vs {}", got[i].top[0].0); }
        }
        extra += used.iter().filter(|u| !**u).count();
    }
    println!("{} images: {boxes} boxes matched (min IoU {min_iou:.4}, max confidence difference {max_conf:.1e}), {missing} missing, {extra} extra; \
              same final label {same} of {classified}, max score difference {max_score:.1e}", list.len());
    println!("speed: {:.2} images per second ({:.2} s for {} images, including reading the JPEGs)", list.len() as f64 / secs, secs, list.len());
    // Where the time goes, on a second pass.
    let (mut t_load, mut t_det, mut t_clf, mut crops) = (0.0, 0.0, 0.0, 0usize);
    for img in list {
        let t = Instant::now();
        let im = camtrap::load(&images.join(img["file"].as_str().unwrap()))?;
        t_load += t.elapsed().as_secs_f64();
        let t = Instant::now();
        let d = model.detect(&im)?;
        t_det += t.elapsed().as_secs_f64();
        let b: Vec<[f64; 4]> = d.iter().filter(|x| x.conf >= camtrap::CLASSIFY_FROM).map(|x| x.bbox).collect();
        let t = Instant::now();
        model.classify(&im, &b)?;
        t_clf += t.elapsed().as_secs_f64();
        crops += b.len();
    }
    let n = list.len() as f64;
    println!("per image: reading {:.3} s, detector {:.3} s (incl. resize), classifier {:.3} s per box ({} boxes)", t_load / n, t_det / n, t_clf / crops.max(1) as f64, crops);
    Ok(())
}

fn iou(a: &[f64], b: &[f64; 4]) -> f64 {
    let (ax2, ay2, bx2, by2) = (a[0] + a[2], a[1] + a[3], b[0] + b[2], b[1] + b[3]);
    let w = (ax2.min(bx2) - a[0].max(b[0])).max(0.0);
    let h = (ay2.min(by2) - a[1].max(b[1])).max(0.0);
    let i = w * h;
    i / (a[2] * a[3] + b[2] * b[3] - i)
}
