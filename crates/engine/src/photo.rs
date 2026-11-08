//! What a camera-trap photo says about itself (EXIF): when it was taken, where, which camera, and how to turn it.
use chrono::NaiveDateTime;
use exif::{In, Tag, Value};
use std::path::Path;

pub const IMAGE_EXT: &[&str] = &["jpg", "jpeg", "png"];

pub fn is_image(p: &Path) -> bool { p.extension().and_then(|e| e.to_str()).is_some_and(|e| IMAGE_EXT.iter().any(|a| e.eq_ignore_ascii_case(a))) }

#[derive(Debug, Default, Clone, PartialEq)]
pub struct PhotoMeta {
    /// DateTimeOriginal (local camera time), else DateTime.
    pub taken_local: Option<NaiveDateTime>,
    /// OffsetTimeOriginal in minutes, when the camera wrote it.
    pub tz_offset_min: Option<i32>,
    pub lat: Option<f64>, pub lon: Option<f64>,
    pub make: Option<String>, pub model: Option<String>, pub serial: Option<String>,
    pub width: Option<u32>, pub height: Option<u32>,
    pub orientation: u16,
}

fn exif_of(bytes: &[u8]) -> Option<exif::Exif> { exif::Reader::new().read_from_container(&mut std::io::Cursor::new(bytes)).ok() }

fn text(e: &exif::Exif, tag: Tag) -> Option<String> {
    let f = e.get_field(tag, In::PRIMARY)?;
    match &f.value {
        Value::Ascii(v) => v.first().map(|s| String::from_utf8_lossy(s).trim().trim_matches('\0').to_string()).filter(|s| !s.is_empty()),
        _ => Some(f.display_value().to_string()).filter(|s| !s.is_empty()),
    }
}

fn uint(e: &exif::Exif, tag: Tag) -> Option<u32> { e.get_field(tag, In::PRIMARY).and_then(|f| f.value.get_uint(0)) }

fn coord(e: &exif::Exif, tag: Tag, reference: Tag) -> Option<f64> {
    let f = e.get_field(tag, In::PRIMARY)?;
    let Value::Rational(v) = &f.value else { return None };
    if v.len() < 3 || v.iter().any(|r| r.denom == 0) { return None; }
    let d = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
    let r = text(e, reference).unwrap_or_default();
    let d = if r.starts_with('S') || r.starts_with('W') { -d } else { d };
    (d.is_finite() && d.abs() <= 180.0 && d != 0.0).then_some(d)
}

/// "+05:30" or "-08:00" to minutes.
fn offset(s: &str) -> Option<i32> {
    let s = s.trim();
    let sign = match s.chars().next()? { '+' => 1, '-' => -1, _ => return None };
    let (h, m) = s[1..].split_once(':')?;
    Some(sign * (h.parse::<i32>().ok()? * 60 + m.parse::<i32>().ok()?))
}

/// EXIF orientation (1 to 8, 1 when absent) of an image file's bytes.
pub fn orientation(bytes: &[u8]) -> u16 { exif_of(bytes).and_then(|e| uint(&e, Tag::Orientation)).map(|v| v as u16).unwrap_or(1) }

pub fn read_bytes(bytes: &[u8]) -> PhotoMeta {
    let Some(e) = exif_of(bytes) else { return PhotoMeta { orientation: 1, ..Default::default() } };
    let when = text(&e, Tag::DateTimeOriginal).or_else(|| text(&e, Tag::DateTime));
    let taken_local = when.and_then(|s| NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S").ok()
        .or_else(|| NaiveDateTime::parse_from_str(s.trim(), "%Y:%m:%d %H:%M:%S").ok()));
    let (lat, lon) = match (coord(&e, Tag::GPSLatitude, Tag::GPSLatitudeRef), coord(&e, Tag::GPSLongitude, Tag::GPSLongitudeRef)) {
        (Some(a), Some(o)) if a.abs() <= 90.0 => (Some(a), Some(o)),
        _ => (None, None),
    };
    PhotoMeta {
        taken_local,
        tz_offset_min: text(&e, Tag::OffsetTimeOriginal).or_else(|| text(&e, Tag::OffsetTime)).and_then(|s| offset(&s)),
        lat, lon,
        make: text(&e, Tag::Make), model: text(&e, Tag::Model), serial: text(&e, Tag::BodySerialNumber),
        width: uint(&e, Tag::PixelXDimension), height: uint(&e, Tag::PixelYDimension),
        orientation: uint(&e, Tag::Orientation).map(|v| v as u16).unwrap_or(1),
    }
}

/// Reads only the start of the file: EXIF sits in the first 64 KB of a JPEG.
pub fn read(path: &Path) -> PhotoMeta {
    use std::io::Read;
    let mut buf = Vec::new();
    if let Ok(f) = std::fs::File::open(path) { let _ = f.take(256 * 1024).read_to_end(&mut buf); }
    let mut m = read_bytes(&buf);
    if m.width.is_none() {
        if let Ok((w, h)) = image::image_dimensions(path) { m.width = Some(w); m.height = Some(h); }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offsets_and_types() {
        assert_eq!(offset("+05:30"), Some(330));
        assert_eq!(offset("-08:00"), Some(-480));
        assert_eq!(offset("Z"), None);
        assert!(is_image(Path::new("/a/IMG_0001.JPG")) && is_image(Path::new("b.png")) && !is_image(Path::new("c.wav")));
        assert_eq!(read_bytes(b"not an image").orientation, 1);
    }
}
