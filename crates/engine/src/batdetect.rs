//! Studio's connector for BatDetect2 (Mac Aodha et al., UK bat model). The BatDetect2 processing itself (spectrogram,
//! network run, call decoding, resampling) is in crates/batdetect2, which is CC BY-NC 4.0 like BatDetect2; this file is
//! Studio's own code (Apache-2.0): it loads the files a user made, names the labels and plugs the model into Studio.
use crate::models::{Call, Classifier, Label, ModelInfo, Scored};
use anyhow::{bail, Context, Result};
use std::path::Path;

pub use wlnw_batdetect2::{decode, Config, Found, PolyResampler, Preprocessor, CITATION, LICENCE, LICENCE_URL, PAPER_URL, REPO_URL};

pub const ID: &str = "batdetect2-uk";
/// Recordings at or above this rate (after time expansion) are bat recordings: BatDetect2 is their default model.
pub const ULTRASONIC_RATE: u32 = 192_000;

pub struct BatDetect {
    net: wlnw_batdetect2::Network,
    labels: Vec<Label>,
    info: ModelInfo,
}

impl BatDetect {
    pub fn load(dir: &Path) -> Result<Self> {
        let cfg: Config = serde_json::from_str(&std::fs::read_to_string(dir.join("batdetect2.json")).context("batdetect2.json missing: see crates/batdetect2/README.md for how the files are made")?)?;
        let labels: Vec<Label> = std::fs::read_to_string(dir.join("labels.txt")).context("labels.txt")?.lines().filter(|l| !l.trim().is_empty()).map(|l| {
            let (s, c) = l.split_once('_').unwrap_or((l, ""));
            Label { scientific: s.to_string(), common: c.to_string() }
        }).collect();
        if labels.is_empty() { bail!("labels.txt is empty"); }
        let clip = cfg.clip_len();
        Ok(BatDetect {
            info: ModelInfo {
                id: ID.into(), name: "BatDetect2".into(), version: "UK 2.0".into(), licence: LICENCE.into(),
                licence_url: LICENCE_URL.into(), citation: CITATION.into(),
                sample_rate: cfg.samplerate, window: clip, min_len: clip, embeds: false, location_filter: false, speed_x: 7.0,
            },
            net: wlnw_batdetect2::Network::new(cfg, crate::birdnet::session(dir.join("batdetect2_uk.onnx"), crate::birdnet::threads())?, labels.len()),
            labels,
        })
    }

    /// Every call in each clip (clips of exactly `clip_len` samples at 256 kHz).
    pub fn find(&self, clips: &[Vec<f32>]) -> Result<Vec<Vec<Found>>> { self.net.find(clips) }
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
