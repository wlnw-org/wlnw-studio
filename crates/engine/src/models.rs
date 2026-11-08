//! Model packs: every classifier Studio can run looks the same to the rest of the engine. A pack is a folder with the
//! model file(s), its labels and its licence; `available` lists the packs found in the models folder.
use anyhow::Result;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    /// Stable id stored with every detection: "birdnet-2.4", "perch-2.0", "wnc".
    pub id: String,
    pub name: String,
    pub version: String,
    pub licence: String,
    pub licence_url: String,
    pub citation: String,
    pub sample_rate: u32,
    /// Window length in samples (BirdNET 3 s at 48 kHz, Perch 5 s at 32 kHz).
    pub window: usize,
    /// A last piece shorter than this is dropped, longer ones are padded.
    pub min_len: usize,
    /// True when the model also returns an embedding per window (for the Wildlife Network classifier and "find more like this").
    pub embeds: bool,
    /// True when the model can say which species to expect at a place and week.
    pub location_filter: bool,
    /// Rough speed on an 8-core laptop, in multiples of real time, so the app can estimate durations.
    pub speed_x: f64,
}

/// One scored window: confidence per label (0 to 1), and the embedding when the model has one.
pub struct Scored { pub scores: Vec<f32>, pub embedding: Option<Vec<f32>> }

#[derive(Debug, Clone)]
pub struct Label { pub scientific: String, pub common: String }

/// One call found inside a window by a model that detects individual calls (BatDetect2). Times in seconds from the
/// window start, as the model hears them (before any time expansion is undone).
#[derive(Debug, Clone, PartialEq)]
pub struct Call { pub start: f64, pub end: f64, pub label: usize, pub confidence: f32, pub detection: f32, pub low_hz: f64, pub high_hz: f64 }

pub trait Classifier: Send + Sync {
    fn info(&self) -> &ModelInfo;
    fn labels(&self) -> &[Label];
    fn predict(&self, windows: &[Vec<f32>]) -> Result<Vec<Scored>>;
    /// True for models that return individual calls with their own start and end (`calls`) instead of one score per window.
    fn finds_calls(&self) -> bool { false }
    /// The calls in each window, for models where `finds_calls` is true.
    fn calls(&self, _windows: &[Vec<f32>]) -> Result<Vec<Vec<Call>>> { anyhow::bail!("{} scores windows, it does not find calls", self.info().name) }
    /// Probability that each label occurs at this place in this BirdNET week (1 to 48); None when the model has no location model.
    fn expected(&self, _lat: f64, _lon: f64, _week: i32) -> Result<Option<Vec<f32>>> { Ok(None) }
    /// Detections below this are not stored (they would only fill the project).
    fn floor(&self) -> f32 { 0.1 }
}

/// Every pack found in `dir`. A pack that fails to load is reported, not fatal.
pub fn available(dir: &Path) -> (Vec<Arc<dyn Classifier>>, Vec<String>) { available_in(&[dir.to_path_buf()]) }

/// Every pack found in any of `dirs` (the first folder holding a pack wins), e.g. the app's downloads folder and its bundle.
pub fn available_in(dirs: &[std::path::PathBuf]) -> (Vec<Arc<dyn Classifier>>, Vec<String>) {
    let mut ok: Vec<Arc<dyn Classifier>> = Vec::new();
    let mut errors = Vec::new();
    if let Some(d) = dirs.iter().find(|d| d.join("birdnet_v24_cnn.onnx").exists()) {
        match crate::birdnet::BirdNet::load(d) { Ok(m) => ok.push(Arc::new(m)), Err(e) => errors.push(format!("BirdNET: {e:#}")) }
    }
    if let Some(d) = dirs.iter().map(|d| d.join("perch_v2")).find(|d| d.join("perch_v2.onnx").exists() && d.join("labels.csv").exists()) {
        let common = ok.first().map(|b| b.labels().to_vec()).unwrap_or_default();
        match crate::perch::Perch::load(&d, &common) { Ok(m) => ok.push(Arc::new(m)), Err(e) => errors.push(format!("Perch: {e:#}")) }
    }
    if let Some(d) = dirs.iter().map(|d| d.join("batdetect2")).find(|d| d.join("batdetect2_uk.onnx").exists()) {
        match crate::batdetect::BatDetect::load(&d) { Ok(m) => ok.push(Arc::new(m)), Err(e) => errors.push(format!("BatDetect2: {e:#}")) }
    }
    (ok, errors)
}

/// Where BirdNET is weakest (Africa and South Asia), Perch and the Wildlife Network classifier are the default.
/// Boxes are deliberately broad: Africa with Madagascar, and South Asia from Pakistan to Myanmar with Sri Lanka.
pub fn perch_first(lat: f64, lon: f64) -> bool {
    // Africa, without the Arabian Peninsula and the Levant east of the Red Sea and Suez.
    let africa = (-35.0..=37.5).contains(&lat) && (-18.0..=52.0).contains(&lon) && !(lat > 12.5 && lon > 43.0) && !(lat > 22.0 && lon > 35.0);
    let south_asia = (5.0..=37.0).contains(&lat) && (60.0..=97.5).contains(&lon);
    africa || south_asia
}

/// The models to run for a recorder: BatDetect2 for ultrasonic recordings (`rate`, after time expansion, at or above
/// 192 kHz) when it is installed; otherwise Perch first in Africa and South Asia (BirdNET too, as a second opinion), BirdNET elsewhere.
pub fn default_models(lat: Option<f64>, lon: Option<f64>, rate: Option<u32>, have: &[String]) -> Vec<String> {
    let has = |id: &str| have.iter().any(|h| h == id);
    if rate.is_some_and(|r| r >= crate::batdetect::ULTRASONIC_RATE) && has(crate::batdetect::ID) { return vec![crate::batdetect::ID.to_string()]; }
    let mut out = Vec::new();
    match (lat, lon) {
        (Some(la), Some(lo)) if perch_first(la, lo) && has("perch-2.0") => { out.push("perch-2.0".to_string()); if has("birdnet-2.4") { out.push("birdnet-2.4".into()); } }
        _ => { if has("birdnet-2.4") { out.push("birdnet-2.4".into()); } else if has("perch-2.0") { out.push("perch-2.0".into()); } }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regions() {
        assert!(perch_first(13.08, 80.27));   // Chennai
        assert!(perch_first(-8.75, 26.75));   // southern DR Congo
        assert!(perch_first(-19.4, 15.9));    // northern Namibia
        assert!(perch_first(-1.29, 36.82));   // Nairobi
        assert!(!perch_first(51.5, -0.12));   // London
        assert!(!perch_first(9.93, -84.08));  // San José, Costa Rica
        assert!(!perch_first(24.7, 46.7));    // Riyadh
        assert!(perch_first(30.04, 31.24));   // Cairo
        let have = vec!["birdnet-2.4".to_string(), "perch-2.0".to_string()];
        assert_eq!(default_models(Some(13.0), Some(80.0), None, &have), vec!["perch-2.0", "birdnet-2.4"]);
        assert_eq!(default_models(Some(51.5), Some(-0.1), Some(48_000), &have), vec!["birdnet-2.4"]);
        assert_eq!(default_models(None, None, None, &have), vec!["birdnet-2.4"]);
        // Ultrasonic recordings go to BatDetect2 when it is installed, to the bird models when not.
        assert_eq!(default_models(Some(51.5), Some(-0.1), Some(256_000), &have), vec!["birdnet-2.4"]);
        let have = vec!["birdnet-2.4".to_string(), "perch-2.0".to_string(), "batdetect2-uk".to_string()];
        assert_eq!(default_models(Some(51.5), Some(-0.1), Some(256_000), &have), vec!["batdetect2-uk"]);
        assert_eq!(default_models(Some(13.0), Some(80.0), Some(192_000), &have), vec!["batdetect2-uk"]);
        assert_eq!(default_models(Some(51.5), Some(-0.1), Some(96_000), &have), vec!["birdnet-2.4"]);
    }
}
