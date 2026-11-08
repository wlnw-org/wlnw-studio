//! BirdNET V2.4: the two mel spectrograms computed here (bit-for-bit the model's own layers, see docs/SPIKE.md)
//! and the network in ONNX Runtime on the CPU. Plus BirdNET's location model for the species expected at a place and week.
use anyhow::{Context, Result};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use rayon::prelude::*;
use realfft::{num_complex::Complex, RealFftPlanner, RealToComplex};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const FRAMES: usize = 511;
const MELS: usize = 96;
pub const BATCH: usize = 32;
/// BirdNET-Analyzer's location filter threshold.
pub const RANGE_THRESHOLD: f32 = 0.03;

pub use crate::models::Label;
use crate::models::{Classifier, ModelInfo, Scored};

struct Mel { n: usize, hop: usize, win: Vec<f32>, rows: Vec<(usize, Vec<f32>)>, bins: usize, p: f32, fft: Arc<dyn RealToComplex<f32>> }

impl Mel {
    fn new(n: usize, hop: usize, win: Vec<f32>, m: Vec<f32>, p: f32, planner: &mut RealFftPlanner<f32>) -> Self {
        let bins = n / 2 + 1;
        let rows = (0..MELS).map(|j| {
            let r = &m[j * bins..(j + 1) * bins];
            let a = r.iter().position(|&v| v != 0.0).unwrap_or(0);
            let b = r.iter().rposition(|&v| v != 0.0).map_or(a, |i| i + 1);
            (a, r[a..b].to_vec())
        }).collect();
        Mel { n, hop, win, rows, bins, p, fft: planner.plan_fft_forward(n) }
    }
    fn compute(&self, s: &[f32], out: &mut [f32], ch: usize) {
        let mut buf = vec![0f32; self.n];
        let mut spec = vec![Complex::new(0f32, 0f32); self.bins];
        let mut scratch = self.fft.make_scratch_vec();
        for f in 0..FRAMES {
            let start = f * self.hop;
            for i in 0..self.n { buf[i] = s[start + i] * self.win[i]; }
            self.fft.process_with_scratch(&mut buf, &mut spec, &mut scratch).expect("fft sizes");
            for (j, (a, w)) in self.rows.iter().enumerate() {
                let acc: f32 = w.iter().zip(&spec[*a..*a + w.len()]).map(|(x, c)| x * c.re).sum();
                out[((MELS - 1 - j) * FRAMES + f) * 2 + ch] = (acc * acc).powf(self.p);
            }
        }
    }
}

pub struct BirdNet {
    mel1: Mel,
    mel2: Mel,
    cnn: Mutex<Session>,
    meta: Mutex<Session>,
    pub labels: Vec<Label>,
    info: ModelInfo,
}

fn read_f32(path: &Path) -> Result<Vec<f32>> {
    let b = std::fs::read(path).with_context(|| format!("model file missing: {}", path.display()))?;
    Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

fn env() -> Result<&'static ort::environment::Environment> {
    // ONNX Runtime allows one environment per process.
    static ENV: std::sync::OnceLock<ort::environment::Environment> = std::sync::OnceLock::new();
    if let Some(e) = ENV.get() { return Ok(e); }
    let e = ort::init().with_name("wlnw-studio").build()?;
    Ok(ENV.get_or_init(|| e))
}

/// Asks macOS to run this thread, and the model threads it creates, on the fast cores. A GUI app's worker threads can
/// otherwise get a low priority (and App Nap when the window is in the background), which made the app about 5 times
/// slower than the command line on the same files. No effect on other systems.
pub fn boost_thread() {
    #[cfg(target_os = "macos")]
    {
        extern "C" { fn pthread_set_qos_class_self_np(qos: u32, relative_priority: i32) -> i32; }
        const QOS_CLASS_USER_INITIATED: u32 = 0x19;
        unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INITIATED, 0); }
    }
}

/// Call once at start-up: the shared worker pool (spectrograms, decoding) runs at the same raised priority.
pub fn init_threads() {
    let _ = rayon::ThreadPoolBuilder::new().start_handler(|_| boost_thread()).build_global();
}

pub(crate) fn threads() -> usize { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) }

pub(crate) fn session(path: PathBuf, threads: usize) -> Result<Session> {
    boost_thread(); // the session's worker threads take the priority of the thread that creates them
    let env = env()?;
    // CPU only: the "max efficiency" device policy of newer ONNX Runtime APIs gave wrong scores on macOS (docs/SPIKE.md).
    Ok(Session::builder(env)?
        .with_optimization_level(GraphOptimizationLevel::All).unwrap_or_else(|e| e.recover())
        .with_intra_threads(threads).unwrap_or_else(|e| e.recover())
        .commit_from_file(&path).with_context(|| format!("model {}", path.display()))?)
}

fn num(json: &str, key: &str) -> Result<f32> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    Ok(v[key].as_f64().with_context(|| format!("mel.json: {key}"))? as f32)
}

impl BirdNet {
    pub fn load(dir: &Path) -> Result<Self> {
        let cfg = std::fs::read_to_string(dir.join("mel.json")).context("mel.json missing: see docs/SPIKE.md for how the model files are made")?;
        let mut planner = RealFftPlanner::<f32>::new();
        let mel1 = Mel::new(2048, 278, read_f32(&dir.join("w1.f32"))?, read_f32(&dir.join("M1.f32"))?, num(&cfg, "p1")?, &mut planner);
        let mel2 = Mel::new(1024, 280, read_f32(&dir.join("w2.f32"))?, read_f32(&dir.join("M2.f32"))?, num(&cfg, "p2")?, &mut planner);
        let labels = std::fs::read_to_string(dir.join("labels.txt"))?.lines().map(|l| {
            let (s, c) = l.split_once('_').unwrap_or((l, l));
            Label { scientific: s.to_string(), common: c.to_string() }
        }).collect();
        let threads = threads();
        Ok(BirdNet {
            mel1, mel2, labels,
            cnn: Mutex::new(session(dir.join("birdnet_v24_cnn.onnx"), threads)?),
            meta: Mutex::new(session(dir.join("birdnet_v24_meta.onnx"), 1)?),
            info: ModelInfo {
                id: "birdnet-2.4".into(), name: "BirdNET".into(), version: "2.4".into(), licence: "CC-BY-NC-SA-4.0".into(),
                licence_url: "https://creativecommons.org/licenses/by-nc-sa/4.0/".into(),
                citation: "Kahl et al. (2021), BirdNET: A deep learning solution for avian diversity monitoring, Ecological Informatics 61, doi:10.1016/j.ecoinf.2021.101236".into(),
                sample_rate: 48_000, window: 144_000, min_len: 48_000, embeds: false, location_filter: true, speed_x: 350.0,
            },
        })
    }

    /// Confidence (0 to 1) for every label, for each 3 s window.
    fn scores(&self, chunks: &[Vec<f32>]) -> Result<Vec<Vec<f32>>> {
        let block = MELS * FRAMES * 2;
        let mut input = vec![0f32; chunks.len() * block];
        input.par_chunks_mut(block).zip(chunks.par_iter()).for_each(|(dst, c)| {
            let (mn, mx) = c.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
            let s: Vec<f32> = c.iter().map(|&v| (((v - mn) / ((mx - mn) + 1e-6)) - 0.5) * 2.0).collect();
            self.mel1.compute(&s, dst, 0);
            self.mel2.compute(&s, dst, 1);
        });
        let x = Tensor::from_array(([chunks.len(), MELS, FRAMES, 2], input))?;
        let mut cnn = self.cnn.lock().unwrap();
        let out = cnn.run(ort::inputs![x])?;
        let (_, logits) = out[0].try_extract_tensor::<f32>()?;
        let n = self.labels.len();
        Ok((0..chunks.len()).map(|b| logits[b * n..(b + 1) * n].iter().map(|&l| 1.0 / (1.0 + (-l.clamp(-20.0, 20.0)).exp())).collect()).collect())
    }

    /// Probability that each species occurs at this place in this BirdNET week (1 to 48).
    fn location(&self, lat: f64, lon: f64, week: i32) -> Result<Vec<f32>> {
        let x = Tensor::from_array(([1usize, 3], vec![lat as f32, lon as f32, week as f32]))?;
        let mut meta = self.meta.lock().unwrap();
        let out = meta.run(ort::inputs![x])?;
        let (_, p) = out[0].try_extract_tensor::<f32>()?;
        Ok(p.to_vec())
    }
}

impl Classifier for BirdNet {
    fn info(&self) -> &ModelInfo { &self.info }
    fn labels(&self) -> &[Label] { &self.labels }
    fn predict(&self, windows: &[Vec<f32>]) -> Result<Vec<Scored>> {
        Ok(self.scores(windows)?.into_iter().map(|scores| Scored { scores, embedding: None }).collect())
    }
    fn expected(&self, lat: f64, lon: f64, week: i32) -> Result<Option<Vec<f32>>> { self.location(lat, lon, week).map(Some) }
}
