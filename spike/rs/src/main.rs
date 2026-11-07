// Speed and accuracy test: BirdNET V2.4 as a Rust mel spectrogram + the CNN in ONNX Runtime, on CPU.
// Usage: spike <models dir> <out.csv> <wav files...>
use anyhow::{Context, Result};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use rayon::prelude::*;
use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex;
use std::io::Write;
use std::time::Instant;

const SR: usize = 48_000;
const CHUNK: usize = 144_000; // 3 s
const MIN_LEN: usize = 48_000; // last chunk at least 1 s, padded with zeros (as BirdNET-Analyzer)
fn batch_size() -> usize { std::env::var("BATCH").ok().and_then(|v| v.parse().ok()).unwrap_or(32) }
const FRAMES: usize = 511;
const MELS: usize = 96;

fn read_f32(path: &str) -> Result<Vec<f32>> {
    let b = std::fs::read(path).with_context(|| path.to_string())?;
    Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

struct Mel { n: usize, hop: usize, win: Vec<f32>, rows: Vec<(usize, Vec<f32>)>, bins: usize, p: f32 }

impl Mel {
    fn new(n: usize, hop: usize, win: Vec<f32>, m: Vec<f32>, p: f32) -> Self {
        let bins = n / 2 + 1;
        // Each mel filter is a short run of non-zero weights: keep only that run.
        let rows = (0..MELS).map(|j| {
            let r = &m[j * bins..(j + 1) * bins];
            let a = r.iter().position(|&v| v != 0.0).unwrap_or(0);
            let b = r.iter().rposition(|&v| v != 0.0).map_or(a, |i| i + 1);
            (a, r[a..b].to_vec())
        }).collect();
        Mel { n, hop, win, rows, bins, p }
    }
    /// Writes 96 x 511 values into channel ch of a (96, 511, 2) block.
    fn compute(&self, s: &[f32], fft: &dyn RealToComplex<f32>, out: &mut [f32], ch: usize) {
        let mut buf = vec![0f32; self.n];
        let mut spec = vec![Complex::new(0f32, 0f32); self.bins];
        let mut scratch = fft.make_scratch_vec();
        for f in 0..FRAMES {
            let start = f * self.hop;
            for i in 0..self.n { buf[i] = s[start + i] * self.win[i]; }
            fft.process_with_scratch(&mut buf, &mut spec, &mut scratch).unwrap();
            for (j, (a, w)) in self.rows.iter().enumerate() {
                // The model casts the complex STFT to float: real part only.
                let acc: f32 = w.iter().zip(&spec[*a..*a + w.len()]).map(|(x, c)| x * c.re).sum();
                out[((MELS - 1 - j) * FRAMES + f) * 2 + ch] = (acc * acc).powf(self.p);
            }
        }
    }
}

fn normalise(c: &[f32]) -> Vec<f32> {
    let (mn, mx) = c.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    c.iter().map(|&v| (((v - mn) / ((mx - mn) + 1e-6)) - 0.5) * 2.0).collect()
}

fn read_wav(path: &str) -> Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    anyhow::ensure!(spec.sample_rate as usize == SR, "spike only handles 48 kHz, got {}", spec.sample_rate);
    let ch = spec.channels as usize;
    let s: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => { let scale = (1i64 << (spec.bits_per_sample - 1)) as f32; r.samples::<i32>().step_by(ch).map(|x| x.unwrap() as f32 / scale).collect() }
        hound::SampleFormat::Float => r.samples::<f32>().step_by(ch).map(|x| x.unwrap()).collect(),
    };
    Ok(s)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dir = &args[1];
    let out_path = &args[2];
    let files = &args[3..];
    let cfg: std::collections::HashMap<String, f64> = {
        let t = std::fs::read_to_string(format!("{dir}/mel.json"))?;
        t.trim_matches(|c| c == '{' || c == '}' || c == '\n').split(',').map(|kv| {
            let mut it = kv.split(':');
            (it.next().unwrap().trim().trim_matches('"').to_string(), it.next().unwrap().trim().parse().unwrap())
        }).collect()
    };
    let mel1 = Mel::new(2048, 278, read_f32(&format!("{dir}/w1.f32"))?, read_f32(&format!("{dir}/M1.f32"))?, cfg["p1"] as f32);
    let mel2 = Mel::new(1024, 280, read_f32(&format!("{dir}/w2.f32"))?, read_f32(&format!("{dir}/M2.f32"))?, cfg["p2"] as f32);
    let labels: Vec<String> = std::fs::read_to_string(format!("{dir}/labels.txt"))?.lines().map(String::from).collect();
    let threads = std::thread::available_parallelism()?.get();
    let env = ort::init().build()?;
    let mut session = Session::builder(&env)?
        .with_optimization_level(match std::env::var("OPT").as_deref() { Ok("0") => GraphOptimizationLevel::Disable, Ok("1") => GraphOptimizationLevel::Level1, _ => GraphOptimizationLevel::All }).unwrap_or_else(|e| e.recover())
        .with_intra_threads(std::env::var("THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(threads)).unwrap_or_else(|e| e.recover())
        .commit_from_file(format!("{dir}/birdnet_v24_cnn.onnx"))?;
    let mut planner = RealFftPlanner::<f32>::new();
    let (f1, f2) = (planner.plan_fft_forward(2048), planner.plan_fft_forward(1024));
    let mut out = std::io::BufWriter::new(std::fs::File::create(out_path)?);
    writeln!(out, "File,Start (s),End (s),Scientific name,Common name,Confidence")?;
    let t0 = Instant::now();
    let (mut audio_s, mut t_mel, mut t_nn, mut dets) = (0f64, 0f64, 0f64, 0usize);
    for file in files {
        let samples = read_wav(file)?;
        audio_s += samples.len() as f64 / SR as f64;
        let starts: Vec<usize> = (0..samples.len()).step_by(CHUNK).filter(|&s| samples.len() - s >= MIN_LEN).collect();
        for batch in starts.chunks(batch_size()) {
            let t = Instant::now();
            let block = MELS * FRAMES * 2;
            let mut input = vec![0f32; batch.len() * block];
            input.par_chunks_mut(block).zip(batch.par_iter()).for_each(|(dst, &s)| {
                let mut c = samples[s..(s + CHUNK).min(samples.len())].to_vec();
                c.resize(CHUNK, 0.0);
                let c = normalise(&c);
                mel1.compute(&c, f1.as_ref(), dst, 0);
                mel2.compute(&c, f2.as_ref(), dst, 1);
            });
            if std::env::var("DUMP").is_ok() && batch[0] == 0 { let b: Vec<u8> = input.iter().flat_map(|v| v.to_le_bytes()).collect(); std::fs::write("dump.f32", b)?; }
            t_mel += t.elapsed().as_secs_f64();
            let t = Instant::now();
            let x = Tensor::from_array(([batch.len(), MELS, FRAMES, 2], input))?;
            let res = session.run(ort::inputs![x])?;
            let (_, logits) = res[0].try_extract_tensor::<f32>()?;
            t_nn += t.elapsed().as_secs_f64();
            let n = labels.len();
            if std::env::var("DUMP").is_ok() && batch[0] == 0 { let b: Vec<u8> = logits.iter().flat_map(|v| v.to_le_bytes()).collect(); std::fs::write("logits.f32", b)?; }
            if std::env::var("DUMP").is_ok() { eprintln!("out len {} batch {} n {}", logits.len(), batch.len(), n); }
            for (b, &s) in batch.iter().enumerate() {
                for k in 0..n {
                    let conf = 1.0 / (1.0 + (-logits[b * n + k].clamp(-20.0, 20.0)).exp());
                    if conf >= 0.1 {
                        let (sci, com) = labels[k].split_once('_').unwrap_or((&labels[k], ""));
                        let st = s as f64 / SR as f64;
                        writeln!(out, "{file},{st:.1},{:.1},{sci},{com},{conf:.4}", st + 3.0)?;
                        dets += 1;
                    }
                }
            }
        }
    }
    let wall = t0.elapsed().as_secs_f64();
    eprintln!("audio {:.1} h, wall {:.1} s, {:.0}x real time, mel {:.1} s, nn {:.1} s, {} detections, {} threads",
        audio_s / 3600.0, wall, audio_s / wall, t_mel, t_nn, dets, threads);
    Ok(())
}
