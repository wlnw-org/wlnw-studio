//! Prints every call BatDetect2 finds in the given recordings as JSON, for comparison with the Python package
//! (docs/SPIKE.md), and times the engine's own process_file on them.
//! Usage: cargo run --release -p wlnw-engine --example batdetect_check -- <models/batdetect2> <out.json> <wav>...
use std::path::Path;
use std::time::Instant;
use wlnw_engine::{audio, batdetect::{BatDetect, PolyResampler}, models::Classifier, run};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let model = BatDetect::load(Path::new(&args[1]))?;
    let info = model.info().clone();
    let mut files = serde_json::Map::new();
    let (mut audio_s, mut wall) = (0f64, 0f64);
    for f in &args[3..] {
        let path = Path::new(f);
        // Same reading as run::process_file for call models: native-rate clips, each resampled on its own.
        let mut s = audio::open_native(path)?;
        let rate = s.source_rate;
        let native = (info.window as f64 / info.sample_rate as f64 * rate as f64) as usize;
        let rs = PolyResampler::new(rate, info.sample_rate);
        let mut rows = Vec::new();
        let mut clips = Vec::new();
        let mut starts = Vec::new();
        while let Some((st, c)) = s.next_window(native, native)? { let mut c = rs.apply(&c); c.resize(info.window, 0.0); starts.push(st); clips.push(c); }
        for (b, calls) in model.find(&clips)?.into_iter().enumerate() {
            let t0 = starts[b] as f64 / rate as f64;
            for c in calls {
                rows.push(serde_json::json!({ "start": t0 + c.start, "end": t0 + c.end, "low": c.low, "high": c.high, "det": c.detection, "cls": c.classes }));
            }
        }
        let t = Instant::now();
        let r = run::process_file(path, &model)?;
        let dt = t.elapsed().as_secs_f64();
        audio_s += r.seconds;
        wall += dt;
        eprintln!("{} {} calls, {} stored rows, {:.2} s audio in {:.3} s", path.file_name().unwrap().to_string_lossy(), rows.len(), r.detections.len(), r.seconds, dt);
        files.insert(path.file_name().unwrap().to_string_lossy().to_string(), serde_json::json!({ "secs": dt, "audio_s": r.seconds, "dets": rows }));
    }
    eprintln!("total {audio_s:.1} s of audio in {wall:.2} s = {:.1}x real time", audio_s / wall);
    std::fs::write(&args[2], serde_json::to_string(&serde_json::json!({ "files": files }))?)?;
    Ok(())
}
