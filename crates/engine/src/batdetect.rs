//! BatDetect2 (Mac Aodha et al., UK bat model, CC BY-NC 4.0): finds each echolocation call in ultrasonic recordings and
//! names the species. The network runs in ONNX Runtime; the spectrogram before it and the call decoding after it are
//! computed here, checked against the Python package on real recordings (docs/SPIKE.md).
//!
//! Per 0.5 s clip at 256 kHz: STFT (Hann, 512 samples, hop 128, centred with reflection), magnitude, 10 to 120 kHz,
//! PCEN, spectral mean subtraction, bilinear resize to 128 x 500. After the network: 9 x 9 peak picking on the
//! detection map, the 50 strongest peaks per clip above 0.01, and a box per peak from the size map.
use crate::models::{Call, Classifier, Label, ModelInfo, Scored};
use anyhow::{bail, Context, Result};
use ort::session::Session;
use ort::value::Tensor;
use rayon::prelude::*;
use realfft::{num_complex::Complex, RealFftPlanner, RealToComplex};
use serde::Deserialize;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const ID: &str = "batdetect2-uk";
/// Recordings at or above this rate (after time expansion) are bat recordings: BatDetect2 is their default model.
pub const ULTRASONIC_RATE: u32 = 192_000;

/// Constants exported with the network (spike/batdetect2_export.py).
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub samplerate: u32,
    pub n_fft: usize,
    pub hop: usize,
    pub low_bin: usize,
    pub high_bin: usize,
    pub pcen_smooth: f64,
    pub pcen_gain: f64,
    pub pcen_bias: f64,
    pub pcen_power: f64,
    pub pcen_eps: f64,
    pub height: usize,
    pub resize_factor: f64,
    pub min_freq: f64,
    pub max_freq: f64,
    pub output_samplerate: f64,
    pub clip_s: f64,
    pub nms_kernel: usize,
    pub detection_threshold: f32,
    pub top_k_per_sec: f64,
    pub time_scale: f64,
    pub frequency_scale: f64,
}

impl Config {
    pub fn clip_len(&self) -> usize { (self.clip_s * self.samplerate as f64).round() as usize }
    pub fn frames(&self) -> usize { self.clip_len() / self.hop + 1 }
    pub fn width(&self) -> usize { (self.resize_factor * self.frames() as f64) as usize }
    pub fn bins(&self) -> usize { self.high_bin - self.low_bin }
}

/// The spectrogram the network sees, as BatDetect2's preprocessor computes it.
pub struct Preprocessor { pub cfg: Config, win: Vec<f32>, fft: Arc<dyn RealToComplex<f32>> }

impl Preprocessor {
    pub fn new(cfg: Config) -> Self {
        let n = cfg.n_fft;
        // torch.hann_window is periodic by default.
        let win = (0..n).map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()) as f32).collect();
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
        Preprocessor { cfg, win, fft }
    }

    /// Magnitude STFT, cropped to the model's band: rows = frequency bins (low first), columns = frames.
    pub fn stft(&self, x: &[f32]) -> Vec<f32> {
        let c = &self.cfg;
        let (n, half, frames, bins) = (c.n_fft, c.n_fft / 2, x.len() / c.hop + 1, c.bins());
        // Centred frames: reflect `half` samples at each end (torch.stft center=True, pad_mode="reflect").
        let len = x.len();
        let mut padded = Vec::with_capacity(len + n);
        for k in (1..=half).rev() { padded.push(x[k.min(len - 1)]); }
        padded.extend_from_slice(x);
        for k in 1..=half { padded.push(x[len.saturating_sub(1 + k)]); }
        let mut out = vec![0f32; bins * frames];
        let mut buf = vec![0f32; n];
        let mut spec = vec![Complex::new(0f32, 0f32); n / 2 + 1];
        let mut scratch = self.fft.make_scratch_vec();
        for f in 0..frames {
            let s = &padded[f * c.hop..f * c.hop + n];
            for i in 0..n { buf[i] = s[i] * self.win[i]; }
            self.fft.process_with_scratch(&mut buf, &mut spec, &mut scratch).expect("fft sizes");
            for (b, v) in spec[c.low_bin..c.high_bin].iter().enumerate() { out[b * frames + f] = v.norm(); }
        }
        out
    }

    /// PCEN along time for each frequency row (BatDetect2's own variant: input scaled by 2^31, first-order IIR smoother).
    pub fn pcen(&self, spec: &mut [f32], frames: usize) {
        let c = &self.cfg;
        let (s, eps, gain, bias, power) = (c.pcen_smooth, c.pcen_eps, c.pcen_gain, c.pcen_bias, c.pcen_power);
        let scale = bias.powf(power);
        for row in spec.chunks_mut(frames) {
            let mut m = 0f64;
            for v in row.iter_mut() {
                let x = *v as f64 * 2f64.powi(31);
                m = s * x + (1.0 - s) * m;
                let smooth = (-gain * (eps.ln() + (m.max(0.0) / eps).ln_1p())).exp();
                *v = (scale * (power * (x * smooth / bias).ln_1p()).exp_m1()) as f32;
            }
        }
    }

    /// Spectral mean subtraction: remove each row's mean over time, keep what is above it.
    pub fn mean_subtract(spec: &mut [f32], frames: usize) {
        for row in spec.chunks_mut(frames) {
            let mean = (row.iter().map(|&v| v as f64).sum::<f64>() / frames as f64) as f32;
            for v in row.iter_mut() { *v = (*v - mean).max(0.0); }
        }
    }

    /// Bilinear resize, as torch.nn.functional.interpolate(mode="bilinear", align_corners=False).
    pub fn resize(spec: &[f32], h: usize, w: usize, oh: usize, ow: usize) -> Vec<f32> {
        let axis = |inp: usize, out: usize| -> Vec<(usize, usize, f32)> {
            let scale = inp as f32 / out as f32;
            (0..out).map(|o| {
                let src = (scale * (o as f32 + 0.5) - 0.5).max(0.0);
                let i0 = (src as usize).min(inp - 1);
                let i1 = if i0 < inp - 1 { i0 + 1 } else { i0 };
                (i0, i1, src - i0 as f32)
            }).collect()
        };
        let (ys, xs) = (axis(h, oh), axis(w, ow));
        let mut out = vec![0f32; oh * ow];
        for (oy, &(y0, y1, ly)) in ys.iter().enumerate() {
            for (ox, &(x0, x1, lx)) in xs.iter().enumerate() {
                let p = |y: usize, x: usize| spec[y * w + x];
                out[oy * ow + ox] = (1.0 - ly) * ((1.0 - lx) * p(y0, x0) + lx * p(y0, x1)) + ly * ((1.0 - lx) * p(y1, x0) + lx * p(y1, x1));
            }
        }
        out
    }

    /// The whole chain for one clip of `clip_len` samples at `samplerate`: height x width values.
    pub fn run(&self, clip: &[f32]) -> Vec<f32> {
        let frames = clip.len() / self.cfg.hop + 1;
        let mut s = self.stft(clip);
        self.pcen(&mut s, frames);
        Self::mean_subtract(&mut s, frames);
        Self::resize(&s, self.cfg.bins(), frames, self.cfg.height, (self.cfg.resize_factor * frames as f64) as usize)
    }
}

/// One call found in a clip, in seconds from the clip start and Hz.
#[derive(Debug, Clone, PartialEq)]
pub struct Found { pub start: f64, pub end: f64, pub low: f64, pub high: f64, pub detection: f32, pub classes: Vec<f32> }

/// Peaks of the detection map (9 x 9 local maxima), the strongest first, and their boxes and class scores.
/// `det`: h x w; `size`: 2 x h x w (width, height); `cls`: k x h x w.
pub fn decode(cfg: &Config, det: &[f32], size: &[f32], cls: &[f32], h: usize, w: usize) -> Vec<Found> {
    let r = (cfg.nms_kernel - 1) / 2;
    // A 2-D max over a square window is a max over rows of a max over columns.
    let mut rowmax = vec![0f32; h * w];
    for y in 0..h { for x in 0..w {
        let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
        rowmax[y * w + x] = det[y * w + a..=y * w + b].iter().cloned().fold(f32::MIN, f32::max);
    } }
    let mut peaks: Vec<(f32, usize)> = Vec::new();
    for y in 0..h { for x in 0..w {
        let v = det[y * w + x];
        if v < cfg.detection_threshold { continue; }
        let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
        let m = (a..=b).map(|yy| rowmax[yy * w + x]).fold(f32::MIN, f32::max);
        if m == v { peaks.push((v, y * w + x)); }
    } }
    peaks.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let duration = w as f64 / cfg.output_samplerate;
    peaks.truncate((cfg.top_k_per_sec * duration) as usize);
    let k = cls.len() / (h * w);
    let band = cfg.max_freq - cfg.min_freq;
    peaks.into_iter().map(|(v, i)| {
        let (y, x) = (i / w, i % w);
        let start = x as f64 / w as f64 * duration;
        let low = y as f64 / h as f64 * band + cfg.min_freq;
        // Box anchored at its bottom-left corner (start time, lowest frequency).
        let (bw, bh) = (size[i] as f64, size[h * w + i] as f64);
        Found { start, end: start + bw / cfg.time_scale, low, high: low + bh / cfg.frequency_scale, detection: v,
            classes: (0..k).map(|c| cls[c * h * w + i]).collect() }
    }).collect()
}

/// scipy.signal.resample_poly with its defaults (Kaiser window, beta 5, half length 10 x max(up, down), zero padding),
/// which BatDetect2 uses to bring every clip to 256 kHz.
pub struct PolyResampler { up: usize, down: usize, half: usize, h: Vec<f64> }

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1f64, 1f64, 1f64);
    loop {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        if term < sum * 1e-17 { return sum; }
        k += 1.0;
    }
}

fn gcd(a: usize, b: usize) -> usize { if b == 0 { a } else { gcd(b, a % b) } }

impl PolyResampler {
    pub fn new(from: u32, to: u32) -> Self {
        let g = gcd(from as usize, to as usize).max(1);
        let (up, down) = (to as usize / g, from as usize / g);
        if up == down { return PolyResampler { up: 1, down: 1, half: 0, h: vec![1.0] }; }
        let max = up.max(down);
        let (half, fc) = (10 * max, 1.0 / max as f64);
        let n = 2 * half + 1;
        // firwin(n, fc, window=("kaiser", 5.0)): windowed sinc, scaled to unit gain at DC, then times `up`.
        let beta = 5.0;
        let mut h: Vec<f64> = (0..n).map(|i| {
            let m = i as f64 - half as f64;
            let x = fc * m;
            let sinc = if x == 0.0 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
            let r = 2.0 * i as f64 / (n - 1) as f64 - 1.0;
            fc * sinc * bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(beta)
        }).collect();
        let sum: f64 = h.iter().sum();
        for v in h.iter_mut() { *v = *v / sum * up as f64; }
        PolyResampler { up, down, half, h }
    }

    /// y[k] = sum_i x[i] h[k down + half - i up], for k < ceil(len x up / down).
    pub fn apply(&self, x: &[f32]) -> Vec<f32> {
        if self.up == 1 && self.down == 1 { return x.to_vec(); }
        let (up, down, half) = (self.up as i64, self.down as i64, self.half as i64);
        let n_out = (x.len() * self.up).div_ceil(self.down);
        let last = self.h.len() as i64 - 1;
        (0..n_out as i64).map(|k| {
            let c = k * down + half;
            // i such that 0 <= c - i up <= last
            let i0 = ((c - last).max(0) + up - 1) / up;
            let i1 = (c / up).min(x.len() as i64 - 1);
            let mut acc = 0f64;
            let mut i = i0;
            while i <= i1 { acc += x[i as usize] as f64 * self.h[(c - i * up) as usize]; i += 1; }
            acc as f32
        }).collect()
    }
}

pub struct BatDetect {
    pre: Preprocessor,
    session: Mutex<Session>,
    labels: Vec<Label>,
    info: ModelInfo,
}

impl BatDetect {
    pub fn load(dir: &Path) -> Result<Self> {
        let cfg: Config = serde_json::from_str(&std::fs::read_to_string(dir.join("batdetect2.json")).context("batdetect2.json missing: see docs/SPIKE.md for how the files are made")?)?;
        let labels: Vec<Label> = std::fs::read_to_string(dir.join("labels.txt")).context("labels.txt")?.lines().filter(|l| !l.trim().is_empty()).map(|l| {
            let (s, c) = l.split_once('_').unwrap_or((l, ""));
            Label { scientific: s.to_string(), common: c.to_string() }
        }).collect();
        if labels.is_empty() { bail!("labels.txt is empty"); }
        let clip = cfg.clip_len();
        Ok(BatDetect {
            info: ModelInfo {
                id: ID.into(), name: "BatDetect2".into(), version: "UK 2.0".into(), licence: "CC-BY-NC-4.0".into(),
                licence_url: "https://creativecommons.org/licenses/by-nc/4.0/".into(),
                citation: "Mac Aodha et al. (2022), Towards a General Approach for Bat Echolocation Detection and Classification, bioRxiv doi:10.1101/2022.12.14.520490".into(),
                sample_rate: cfg.samplerate, window: clip, min_len: clip, embeds: false, location_filter: false, speed_x: 7.0,
            },
            session: Mutex::new(crate::birdnet::session(dir.join("batdetect2_uk.onnx"), crate::birdnet::threads())?),
            pre: Preprocessor::new(cfg),
            labels,
        })
    }

    /// Every call in each clip (clips of exactly `clip_len` samples at 256 kHz).
    pub fn find(&self, clips: &[Vec<f32>]) -> Result<Vec<Vec<Found>>> {
        if clips.is_empty() { return Ok(vec![]); }
        let c = &self.pre.cfg;
        let (h, w) = (c.height, c.width());
        let block = h * w;
        let mut input = vec![0f32; clips.len() * block];
        input.par_chunks_mut(block).zip(clips.par_iter()).try_for_each(|(dst, clip)| -> Result<()> {
            if clip.len() != c.clip_len() { bail!("BatDetect2 clips must be {} samples", c.clip_len()); }
            dst.copy_from_slice(&self.pre.run(clip));
            Ok(())
        })?;
        let x = Tensor::from_array(([clips.len(), 1, h, w], input))?;
        let mut s = self.session.lock().unwrap();
        let out = s.run(ort::inputs!["spec" => x])?;
        let (_, det) = out["detection"].try_extract_tensor::<f32>()?;
        let (_, size) = out["size"].try_extract_tensor::<f32>()?;
        let (_, cls) = out["classes"].try_extract_tensor::<f32>()?;
        let k = self.labels.len();
        Ok((0..clips.len()).into_par_iter().map(|b| decode(c, &det[b * block..(b + 1) * block], &size[b * 2 * block..(b + 1) * 2 * block], &cls[b * k * block..(b + 1) * k * block], h, w)).collect())
    }
}

impl Classifier for BatDetect {
    fn info(&self) -> &ModelInfo { &self.info }
    fn labels(&self) -> &[Label] { &self.labels }
    /// Per clip, each species' best call (for callers that only want window scores).
    fn predict(&self, windows: &[Vec<f32>]) -> Result<Vec<Scored>> {
        Ok(self.find(windows)?.into_iter().map(|calls| {
            let mut scores = vec![0f32; self.labels.len()];
            for f in &calls { for (s, &v) in scores.iter_mut().zip(&f.classes) { *s = s.max(v); } }
            Scored { scores, embedding: None }
        }).collect())
    }
    fn finds_calls(&self) -> bool { true }
    /// One row per call: its top species and that species' score (BatDetect2's class_prob).
    fn calls(&self, windows: &[Vec<f32>]) -> Result<Vec<Vec<Call>>> {
        Ok(self.find(windows)?.into_iter().map(|calls| calls.into_iter().map(|f| {
            let (label, conf) = f.classes.iter().enumerate().fold((0, f32::MIN), |b, (i, &v)| if v > b.1 { (i, v) } else { b });
            Call { start: f.start, end: f.end, label, confidence: conf, detection: f.detection, low_hz: f.low, high_hz: f.high }
        }).collect()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        serde_json::from_str(r#"{"samplerate": 256000, "n_fft": 512, "hop": 128, "low_bin": 20, "high_bin": 240, "pcen_smooth": 0.04876562255935639,
            "pcen_gain": 0.98, "pcen_bias": 2.0, "pcen_power": 0.5, "pcen_eps": 1e-06, "height": 128, "resize_factor": 0.5, "min_freq": 10000,
            "max_freq": 120000, "output_samplerate": 1000.0, "clip_s": 0.5, "nms_kernel": 9, "detection_threshold": 0.01, "top_k_per_sec": 100,
            "time_scale": 1000.0, "frequency_scale": 0.0011636363636363637}"#).unwrap()
    }

    /// The fixture's synthetic clip, rebuilt exactly as tests/fixtures/batdetect2_preprocess.py makes it.
    fn clip() -> Vec<f32> {
        let (sr, n) = (256_000f64, 128_000usize);
        let mut state: u64 = 12345;
        (0..n).map(|i| {
            state = (state * 1103515245 + 12345) % (1 << 31);
            let t = i as f64 / sr;
            let gate = if (i / 6400) % 2 == 0 { 1.0 } else { 0.1 };
            (0.3 * (2.0 * std::f64::consts::PI * (60000.0 * t - 40000.0 * t * t)).sin() * gate + 0.02 * (state as f64 / (1u64 << 31) as f64 - 0.5)) as f32
        }).collect()
    }

    /// Each stage against BatDetect2's own preprocessor (values exported from Python: every 997th value, sum and max).
    #[test]
    fn preprocessing_matches_python() {
        let fx: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/batdetect2_preprocess.json")).unwrap();
        let p = Preprocessor::new(cfg());
        let x = clip();
        let frames = x.len() / 128 + 1;
        let check = |name: &str, got: &[f32], tol: f64| {
            let f = &fx[name];
            let want: Vec<f64> = f["every997"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            let shape: Vec<usize> = f["shape"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize).collect();
            assert_eq!(got.len(), shape[0] * shape[1], "{name} shape");
            let mut worst = 0f64;
            for (i, w) in want.iter().enumerate() { worst = worst.max((got[i * 997] as f64 - w).abs() / w.abs().max(1.0)); }
            let sum: f64 = got.iter().map(|&v| v as f64).sum();
            let rel_sum = (sum - f["sum"].as_f64().unwrap()).abs() / f["sum"].as_f64().unwrap();
            eprintln!("{name}: worst sampled relative difference {worst:.2e}, sum {rel_sum:.2e}");
            assert!(worst < tol, "{name}: worst sampled difference {worst}");
            assert!(rel_sum < tol, "{name}: sum differs by {rel_sum}");
        };
        let mut s = p.stft(&x);
        check("crop", &s, 1e-4);
        p.pcen(&mut s, frames);
        check("pcen", &s, 1e-4);
        Preprocessor::mean_subtract(&mut s, frames);
        check("msub", &s, 1e-4);
        let r = Preprocessor::resize(&s, 220, frames, 128, 500);
        check("resize", &r, 1e-4);
        assert_eq!(p.run(&x), r);
    }

    /// The resampler against scipy's resample_poly as BatDetect2 calls it, from four common bat detector rates.
    #[test]
    fn resampling_matches_python() {
        let fx: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/batdetect2_preprocess.json")).unwrap();
        let x: Vec<f32> = clip()[..1000].to_vec();
        for rate in [192_000u32, 312_500, 384_000, 500_000] {
            let f = &fx["resample"][rate.to_string()];
            let want: Vec<f64> = f["values"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            let got = PolyResampler::new(rate, 256_000).apply(&x);
            assert_eq!(got.len(), want.len(), "{rate} length");
            let worst = got.iter().zip(&want).map(|(g, w)| (*g as f64 - w).abs()).fold(0f64, f64::max);
            eprintln!("resample from {rate}: worst difference {worst:.2e}");
            assert!(worst < 2e-6, "{rate}: worst difference {worst}");
        }
    }

    #[test]
    fn decode_peaks() {
        let c = cfg();
        let (h, w) = (128, 500);
        let mut det = vec![0f32; h * w];
        let mut size = vec![0f32; 2 * h * w];
        let mut cls = vec![0f32; 17 * h * w];
        // Two peaks 3 frames apart: only the stronger survives the 9 x 9 window. A third one far away survives.
        for &(y, x, v) in &[(40usize, 100usize, 0.9f32), (41, 103, 0.5), (90, 400, 0.3)] {
            det[y * w + x] = v;
            size[y * w + x] = 5.0;
            size[h * w + y * w + x] = 23.27;
            cls[13 * h * w + y * w + x] = v;
        }
        let f = decode(&c, &det, &size, &cls, h, w);
        assert_eq!(f.len(), 2);
        assert!((f[0].start - 0.1).abs() < 1e-9 && (f[0].end - 0.105).abs() < 1e-9);
        assert!((f[0].low - (40.0 / 128.0 * 110_000.0 + 10_000.0)).abs() < 1e-6);
        assert!((f[0].high - f[0].low - 20_000.0).abs() < 10.0);
        assert_eq!(f[0].classes[13], 0.9);
        assert!((f[1].start - 0.4).abs() < 1e-9);
    }
}
