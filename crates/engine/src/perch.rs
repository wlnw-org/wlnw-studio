//! Perch 2.0 (Google, Apache 2.0): about 15,000 classes (birds, frogs, insects, mammals), 5 s windows at 32 kHz, and a
//! 1,536-number embedding per window that the Wildlife Network classifier learns from. ONNX conversion by Justin Chu,
//! checked against Google's TensorFlow model (docs/SPIKE.md).
use crate::models::{Classifier, Label, ModelInfo, Scored};
use anyhow::{Context, Result};
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

pub const RATE: u32 = 32_000;
pub const WINDOW: usize = 160_000;
pub const EMBED: usize = 1536;

pub struct Perch { info: ModelInfo, labels: Vec<Label>, session: Mutex<Session> }

impl Perch {
    /// `common`: BirdNET's labels, used for English names (Perch's list has scientific names only).
    pub fn load(dir: &Path, common: &[Label]) -> Result<Self> {
        let names: HashMap<&str, &str> = common.iter().map(|l| (l.scientific.as_str(), l.common.as_str())).collect();
        let labels = std::fs::read_to_string(dir.join("labels.csv")).context("labels.csv")?.lines().skip(1)
            .map(|l| { let s = l.trim().trim_matches('"').to_string(); Label { common: names.get(s.as_str()).map(|c| c.to_string()).unwrap_or_default(), scientific: s } }).collect();
        Ok(Perch {
            info: ModelInfo {
                id: "perch-2.0".into(), name: "Perch".into(), version: "2.0".into(), licence: "Apache-2.0".into(),
                licence_url: "https://www.apache.org/licenses/LICENSE-2.0".into(),
                citation: "van Merriënboer et al. (2025), Perch 2.0: The Bittern Lesson for Bioacoustics, arXiv:2508.04665".into(),
                sample_rate: RATE, window: WINDOW, min_len: RATE as usize, embeds: true, location_filter: false, speed_x: 65.0,
            },
            labels,
            session: Mutex::new(crate::birdnet::session(dir.join("perch_v2.onnx"), crate::birdnet::threads())?),
        })
    }
}

/// perch-hoplite's preparation: remove the mean, scale the peak to 0.25.
fn prepare(w: &[f32]) -> Vec<f32> {
    let mean = w.iter().sum::<f32>() / w.len() as f32;
    let peak = w.iter().fold(0f32, |m, &v| m.max((v - mean).abs()));
    let k = if peak > 0.0 { 0.25 / peak } else { 1.0 };
    w.iter().map(|&v| (v - mean) * k).collect()
}

impl Classifier for Perch {
    fn info(&self) -> &ModelInfo { &self.info }
    fn labels(&self) -> &[Label] { &self.labels }
    fn floor(&self) -> f32 { 0.05 }
    fn predict(&self, windows: &[Vec<f32>]) -> Result<Vec<Scored>> {
        let input: Vec<f32> = windows.iter().flat_map(|w| prepare(w)).collect();
        let x = Tensor::from_array(([windows.len(), WINDOW], input))?;
        let mut s = self.session.lock().unwrap();
        let out = s.run(ort::inputs!["inputs" => x])?;
        let (_, logits) = out["label"].try_extract_tensor::<f32>()?;
        let (_, emb) = out["embedding"].try_extract_tensor::<f32>()?;
        let n = self.labels.len();
        Ok((0..windows.len()).map(|b| {
            // Perch's labels are mutually exclusive per window: a softmax gives the probability of each.
            let z = &logits[b * n..(b + 1) * n];
            let m = z.iter().cloned().fold(f32::MIN, f32::max);
            let e: Vec<f32> = z.iter().map(|v| (v - m).exp()).collect();
            let sum: f32 = e.iter().sum();
            Scored { scores: e.into_iter().map(|v| v / sum).collect(), embedding: Some(emb[b * EMBED..(b + 1) * EMBED].to_vec()) }
        }).collect())
    }
}
