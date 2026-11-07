//! What a recording knows about itself: start time, time zone, place and recorder, from AudioMoth comments,
//! GUANO metadata (Song Meter, Kaleidoscope, many bat detectors) and file names.
use chrono::{NaiveDate, NaiveDateTime};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FileMeta {
    /// Local start time as written by the recorder.
    pub start_local: Option<NaiveDateTime>,
    /// Offset from UTC in minutes, when the file says so.
    pub tz_offset_min: Option<i32>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    /// Where the start time came from: "guano", "audiomoth", "filename".
    pub time_source: Option<String>,
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex { cell.get_or_init(|| Regex::new(pat).unwrap()) }

/// Recorder file names: SongMeter `SMM07000_20251102_054500`, AudioMoth `20251102_054500` (UTC by default), BirdNET-Pi `2025-11-02-birdnet-05:45:00`.
pub fn from_file_name(name: &str) -> Option<(NaiveDateTime, Option<String>)> {
    static A: OnceLock<Regex> = OnceLock::new();
    static B: OnceLock<Regex> = OnceLock::new();
    if let Some(c) = re(&A, r"(?:^|[^0-9])(?:([A-Za-z0-9-]+)_)?(20\d{2})(\d{2})(\d{2})[_T-](\d{2})(\d{2})(\d{2})").captures(name) {
        let d = NaiveDate::from_ymd_opt(c[2].parse().ok()?, c[3].parse().ok()?, c[4].parse().ok()?)?;
        let t = d.and_hms_opt(c[5].parse().ok()?, c[6].parse().ok()?, c[7].parse().ok()?)?;
        let prefix = c.get(1).map(|m| m.as_str().to_string()).filter(|p| !p.chars().all(|ch| ch.is_ascii_digit()));
        return Some((t, prefix));
    }
    if let Some(c) = re(&B, r"(20\d{2})-(\d{2})-(\d{2})\D+(\d{2}):?(\d{2}):?(\d{2})").captures(name) {
        let d = NaiveDate::from_ymd_opt(c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?)?;
        return Some((d.and_hms_opt(c[4].parse().ok()?, c[5].parse().ok()?, c[6].parse().ok()?)?, None));
    }
    None
}

/// "Recorded at 21:00:00 24/02/2019 (UTC+1) by AudioMoth 248F2A045C9D9C0B at medium gain ..."
pub fn parse_audiomoth(comment: &str, m: &mut FileMeta) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    let Some(c) = re(&R, r"Recorded at (\d{2}):(\d{2}):(\d{2}) (\d{2})/(\d{2})/(\d{4}) \(UTC(?:([+-])(\d{1,2})(?::?(\d{2}))?)?\)(?: by AudioMoth ([0-9A-F]+))?").captures(comment) else { return false };
    let d = NaiveDate::from_ymd_opt(c[6].parse().unwrap_or(0), c[5].parse().unwrap_or(0), c[4].parse().unwrap_or(0));
    m.start_local = d.and_then(|d| d.and_hms_opt(c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?));
    let sign = if c.get(7).map(|x| x.as_str()) == Some("-") { -1 } else { 1 };
    let h: i32 = c.get(8).map_or(0, |x| x.as_str().parse().unwrap_or(0));
    let mi: i32 = c.get(9).map_or(0, |x| x.as_str().parse().unwrap_or(0));
    m.tz_offset_min = Some(sign * (h * 60 + mi));
    m.make = Some("Open Acoustic Devices".into());
    m.model = Some("AudioMoth".into());
    m.serial = c.get(10).map(|x| x.as_str().to_string());
    m.time_source = Some("audiomoth".into());
    m.start_local.is_some()
}

/// GUANO: "Key: value" lines (https://guano-md.org). Timestamp is ISO 8601, with or without offset.
pub fn parse_guano(text: &str, m: &mut FileMeta) -> bool {
    let mut found = false;
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "Timestamp" => {
                if let Ok(t) = chrono::DateTime::parse_from_rfc3339(v) {
                    m.start_local = Some(t.naive_local());
                    m.tz_offset_min = Some(t.offset().local_minus_utc() / 60);
                    found = true;
                } else if let Ok(t) = NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S%.f").or_else(|_| NaiveDateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S")) {
                    m.start_local = Some(t);
                    found = true;
                }
                if found { m.time_source = Some("guano".into()); }
            }
            "Loc Position" => {
                let p: Vec<f64> = v.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                if p.len() >= 2 { m.lat = Some(p[0]); m.lon = Some(p[1]); }
            }
            "Make" => m.make = Some(v.to_string()),
            "Model" => m.model = Some(v.to_string()),
            "Serial" => m.serial = Some(v.to_string()),
            _ => {}
        }
    }
    found
}

/// Walks the RIFF chunks of a WAV file (skipping the audio itself) for a GUANO chunk or an AudioMoth comment.
fn wav_chunks(path: &Path, m: &mut FileMeta) -> std::io::Result<()> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let mut head = [0u8; 12];
    f.read_exact(&mut head)?;
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" { return Ok(()); }
    let mut pos = 12u64;
    while pos + 8 <= len {
        f.seek(SeekFrom::Start(pos))?;
        let mut h = [0u8; 8];
        if f.read_exact(&mut h).is_err() { break; }
        let size = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as u64;
        let id = &h[0..4];
        if (id == b"guan" || id == b"LIST" || id == b"ICMT") && size < 1 << 20 {
            let mut buf = vec![0u8; size as usize];
            f.read_exact(&mut buf)?;
            let text = String::from_utf8_lossy(&buf);
            if id == b"guan" { parse_guano(&text, m); } else if text.contains("Recorded at") { parse_audiomoth(&text, m); }
        }
        pos += 8 + size + (size & 1);
    }
    Ok(())
}

/// Everything we can learn about one file, best source first: GUANO, AudioMoth comment, file name.
pub fn read(path: &Path) -> FileMeta {
    let mut m = FileMeta::default();
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("wav")) { let _ = wav_chunks(path, &mut m); }
    if m.start_local.is_none() {
        if let Some((t, prefix)) = path.file_name().and_then(|n| n.to_str()).and_then(from_file_name) {
            m.start_local = Some(t);
            m.time_source = Some("filename".into());
            if m.serial.is_none() { m.serial = prefix; }
        }
    }
    m
}

/// BirdNET's week of the year: 4 per month, 1 to 48.
pub fn birdnet_week(t: NaiveDateTime) -> i32 {
    use chrono::Datelike;
    ((t.month() as i32 - 1) * 4 + ((t.day() as i32 - 1) / 7 + 1).min(4)).clamp(1, 48)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names() {
        let (t, p) = from_file_name("SMM07000_20251102_054500.wav").unwrap();
        assert_eq!(t.to_string(), "2025-11-02 05:45:00");
        assert_eq!(p.as_deref(), Some("SMM07000"));
        let (t, p) = from_file_name("20240501_053000.WAV").unwrap();
        assert_eq!(t.to_string(), "2024-05-01 05:30:00");
        assert_eq!(p, None);
        assert!(from_file_name("2025-11-02-birdnet-05:45:00.mp3").is_some());
        assert!(from_file_name("notes.wav").is_none());
    }
    #[test]
    fn audiomoth() {
        let mut m = FileMeta::default();
        assert!(parse_audiomoth("Recorded at 21:00:00 24/02/2019 (UTC+5:30) by AudioMoth 248F2A045C9D9C0B at medium gain", &mut m));
        assert_eq!(m.start_local.unwrap().to_string(), "2019-02-24 21:00:00");
        assert_eq!(m.tz_offset_min, Some(330));
        assert_eq!(m.serial.as_deref(), Some("248F2A045C9D9C0B"));
        let mut m = FileMeta::default();
        assert!(parse_audiomoth("Recorded at 01:02:03 05/06/2024 (UTC) by AudioMoth 24", &mut m));
        assert_eq!(m.tz_offset_min, Some(0));
    }
    #[test]
    fn guano() {
        let mut m = FileMeta::default();
        assert!(parse_guano("GUANO|Version: 1.0\nMake: Wildlife Acoustics, Inc.\nModel: Song Meter Micro\nSerial: SMU01234\nTimestamp: 2025-06-01T21:00:00+02:00\nLoc Position: -19.41 15.92\n", &mut m));
        assert_eq!(m.tz_offset_min, Some(120));
        assert_eq!(m.lat, Some(-19.41));
        assert_eq!(m.model.as_deref(), Some("Song Meter Micro"));
    }
    #[test]
    fn week() {
        assert_eq!(birdnet_week(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap()), 1);
        assert_eq!(birdnet_week(NaiveDate::from_ymd_opt(2025, 11, 20).unwrap().and_hms_opt(0, 0, 0).unwrap()), 43);
        assert_eq!(birdnet_week(NaiveDate::from_ymd_opt(2025, 12, 31).unwrap().and_hms_opt(0, 0, 0).unwrap()), 48);
    }
}
