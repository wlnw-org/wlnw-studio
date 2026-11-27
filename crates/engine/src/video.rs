//! Videos from camera traps (.mp4, .mov, .avi). Studio reads them with FFmpeg, run as a separate program: the copy the user
//! already has, or one downloaded on first use and checked against its SHA-256 like the model packs. A video's sound track
//! goes through the audio models as a recording (the `files` row keeps the video's own path, so detections are timed from
//! the start of the video); its frames go through the camera-trap models (frames.rs and sightings.rs).
use crate::store::Project;
use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDateTime;
use rusqlite::params;
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::RwLock;

pub const VIDEO: &[&str] = &["mp4", "mov", "avi"];

pub fn is_video(p: &Path) -> bool { p.extension().and_then(|e| e.to_str()).is_some_and(|e| VIDEO.iter().any(|a| e.eq_ignore_ascii_case(a))) }

pub const SCHEMA: &str = r#"
-- One row per video. probed = 0 until FFmpeg has read its length, frame rate and size (a video added before FFmpeg was
-- installed is read on the next run). A video with sound also has a row in files under the same path.
CREATE TABLE IF NOT EXISTS videos (
  id INTEGER PRIMARY KEY, deployment_id INTEGER NOT NULL REFERENCES deployments(id), path TEXT NOT NULL UNIQUE,
  taken_local TEXT, time_source TEXT, bytes INTEGER, duration_s REAL, fps REAL, width INTEGER, height INTEGER,
  audio_rate INTEGER, probed INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS videos_deployment ON videos (deployment_id);
"#;

// ---------------------------------------------------------------------------------------------------------------------
// Finding FFmpeg

/// Where downloaded tools go (the models folder); set once by the app or CLI.
static TOOLS_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn set_tools_dir(dir: &Path) { *TOOLS_DIR.write().unwrap() = Some(dir.to_path_buf()); }

const EXE: &str = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };

/// The folder a downloaded FFmpeg is kept in.
pub fn ffmpeg_dir(models_dir: &Path) -> PathBuf { models_dir.join("ffmpeg") }

/// The FFmpeg Studio uses: WLNW_FFMPEG, then the downloaded copy, then one on the PATH or in the usual Homebrew folders
/// (apps opened from the Finder do not get the shell's PATH).
pub fn ffmpeg() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WLNW_FFMPEG").map(PathBuf::from).filter(|p| p.is_file()) { return Some(p); }
    if let Some(d) = TOOLS_DIR.read().unwrap().as_ref() { let p = ffmpeg_dir(d).join(EXE); if p.is_file() { return Some(p); } }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if cfg!(target_os = "macos") { dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from)); }
    dirs.into_iter().map(|d| d.join(EXE)).find(|p| p.is_file())
}

/// An FFmpeg command with no console window on Windows.
pub fn command() -> Result<Command> {
    let exe = ffmpeg().ok_or_else(|| anyhow!("Studio needs FFmpeg to read videos. Download it on the Recordings screen."))?;
    let mut c = Command::new(exe);
    c.arg("-hide_banner").stdin(Stdio::null());
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
    Ok(c)
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading what a video holds

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct VideoInfo {
    pub duration_s: Option<f64>, pub fps: Option<f64>, pub width: Option<u32>, pub height: Option<u32>,
    /// The sound track's sample rate; None when the video has no sound.
    pub audio_rate: Option<u32>,
    /// The container's creation time as written (cameras write their own clock, so it is taken as local time).
    pub created: Option<NaiveDateTime>,
}

/// Reads what `ffmpeg -i <file>` prints about a file (FFmpeg has no machine-readable form without ffprobe).
pub fn parse_probe(text: &str) -> VideoInfo {
    use std::sync::OnceLock;
    static RE: OnceLock<[regex::Regex; 6]> = OnceLock::new();
    let [dur, size, fps, hz, created, rotation] = RE.get_or_init(|| [
        regex::Regex::new(r"Duration: (\d+):(\d\d):(\d\d(?:\.\d+)?)").unwrap(),
        regex::Regex::new(r", (\d{2,5})x(\d{2,5})[ ,\[]").unwrap(),
        regex::Regex::new(r", ([\d.]+(?:k)?) (?:fps|tbr)").unwrap(),
        regex::Regex::new(r", (\d+) Hz").unwrap(),
        regex::Regex::new(r"creation_time\s*: (\d{4}-\d\d-\d\d)[T ](\d\d:\d\d:\d\d)").unwrap(),
        // Phones film sideways and store a display rotation ("displaymatrix: rotation of -90.00 degrees" in older FFmpeg,
        // "Display Matrix: rotation of 90.00 degrees" in newer).
        regex::Regex::new(r"(?i)display ?matrix: rotation of (-?[\d.]+) degrees").unwrap(),
    ]);
    let mut v = VideoInfo::default();
    if let Some(c) = dur.captures(text) {
        v.duration_s = Some(c[1].parse::<f64>().unwrap_or(0.0) * 3600.0 + c[2].parse::<f64>().unwrap_or(0.0) * 60.0 + c[3].parse::<f64>().unwrap_or(0.0));
    }
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate().filter(|(_, l)| l.trim_start().starts_with("Stream #")) {
        if line.contains(": Video: ") && v.width.is_none() {
            // Cover art in an audio file is a "video" stream too, marked as attached_pic: not a video.
            if line.contains("attached pic") { continue; }
            if let Some(c) = size.captures(line) { v.width = c[1].parse().ok(); v.height = c[2].parse().ok(); }
            v.fps = fps.captures(line).and_then(|c| { let s = &c[1]; if let Some(k) = s.strip_suffix('k') { k.parse::<f64>().ok().map(|x| x * 1000.0) } else { s.parse().ok() } });
            // FFmpeg turns frames upright when it decodes them, so report the size as shown, not as stored.
            let rot = lines[i + 1..].iter().take_while(|l| !l.trim_start().starts_with("Stream #"))
                .find_map(|l| rotation.captures(l)).and_then(|c| c[1].parse::<f64>().ok());
            if rot.is_some_and(|r| (r.abs().round() as i64) % 180 == 90) { std::mem::swap(&mut v.width, &mut v.height); }
        } else if line.contains(": Audio: ") && v.audio_rate.is_none() {
            v.audio_rate = hz.captures(line).and_then(|c| c[1].parse().ok());
        }
    }
    v.created = created.captures(text).and_then(|c| NaiveDateTime::parse_from_str(&format!("{} {}", &c[1], &c[2]), "%Y-%m-%d %H:%M:%S").ok())
        // Cameras without a clock write 1904-01-01 (the MP4 epoch) or 1970.
        .filter(|t| t.and_utc().timestamp() > 946_684_800);
    v
}

pub fn probe(path: &Path) -> Result<VideoInfo> {
    let out = command()?.arg("-i").arg(path).stdout(Stdio::null()).stderr(Stdio::piped()).output()
        .with_context(|| format!("could not run FFmpeg on {}", path.display()))?;
    let text = String::from_utf8_lossy(&out.stderr);
    let v = parse_probe(&text);
    if v.width.is_none() && v.audio_rate.is_none() {
        let why = text.lines().rev().find(|l| !l.trim().is_empty() && !l.contains("At least one output file")).unwrap_or("").trim();
        bail!("FFmpeg could not read {}: {why}", path.display());
    }
    Ok(v)
}

/// Adds one video found by a scan (its deployment chosen by scan.rs). Read now when FFmpeg is there, else on the next run.
pub fn add(tx: &rusqlite::Transaction, dep: i64, path: &Path, bytes: i64) -> Result<bool> {
    let p = path.to_string_lossy().to_string();
    let exists: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM videos WHERE path = ?1)", [&p], |r| r.get(0))?;
    if exists { return Ok(false); }
    // A time in the file name (as cameras and recorders write it) wins over the container's clock.
    let m = crate::meta::read(path);
    tx.execute("INSERT INTO videos (deployment_id, path, taken_local, time_source, bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![dep, p, m.start_local.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()), m.start_local.map(|_| m.time_source.clone().unwrap_or_else(|| "filename".into())), bytes])?;
    if ffmpeg().is_some() { let id = tx.last_insert_rowid(); read_one(tx, id, path)?; }
    Ok(true)
}

/// Reads one video with FFmpeg and, when it has sound, adds it to the recordings.
fn read_one(db: &rusqlite::Connection, id: i64, path: &Path) -> Result<()> {
    let v = match probe(path) {
        Ok(v) => v,
        Err(e) => { db.execute("UPDATE videos SET probed = 2 WHERE id = ?1", [id])?; return Err(e); }
    };
    let created = v.created.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string());
    db.execute("UPDATE videos SET duration_s = ?2, fps = ?3, width = ?4, height = ?5, audio_rate = ?6, probed = 1,
        time_source = CASE WHEN taken_local IS NULL AND ?7 IS NOT NULL THEN 'video' ELSE time_source END, taken_local = COALESCE(taken_local, ?7) WHERE id = ?1",
        params![id, v.duration_s, v.fps, v.width, v.height, v.audio_rate, created])?;
    if let Some(rate) = v.audio_rate {
        db.execute("INSERT OR IGNORE INTO files (deployment_id, path, start_local, time_source, bytes, duration_s, sample_rate)
            SELECT deployment_id, path, taken_local, time_source, bytes, duration_s, ?2 FROM videos WHERE id = ?1", params![id, rate as i64])?;
    }
    Ok(())
}

/// Reads the videos added before FFmpeg was installed. Returns how many were read; one that FFmpeg cannot open is marked
/// and skipped (it stays in the list with no length).
pub fn refresh(p: &Project) -> Result<usize> {
    if ffmpeg().is_none() { return Ok(0); }
    let todo: Vec<(i64, String)> = p.db.prepare("SELECT id, path FROM videos WHERE probed = 0")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let mut n = 0;
    for (id, path) in todo { if read_one(&p.db, id, Path::new(&path)).is_ok() { n += 1; } }
    Ok(n)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct VideoCounts { pub total: i64, pub unread: i64, pub with_sound: i64, pub seconds: f64, pub ffmpeg: bool, pub frames_per_s: f64 }

pub fn counts(p: &Project) -> Result<VideoCounts> {
    let (total, unread, with_sound, seconds) = p.db.query_row("SELECT COUNT(*), SUM(probed = 0), SUM(audio_rate IS NOT NULL), COALESCE(SUM(duration_s), 0) FROM videos", [],
        |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0), r.get::<_, Option<i64>>(2)?.unwrap_or(0), r.get(3)?)))?;
    Ok(VideoCounts { total, unread, with_sound, seconds, ffmpeg: ffmpeg().is_some(), frames_per_s: crate::frames::per_s(p) })
}

// ---------------------------------------------------------------------------------------------------------------------
// Sound

/// The sound track as 32-bit float mono samples at `rate`, from `start_s`, streamed from FFmpeg. Silence fills any gap
/// before the first sound packet, so sample 0 is the first frame of the video.
pub fn sound(path: &Path, rate: u32, start_s: f64) -> Result<std::process::Child> {
    let mut c = command()?;
    c.arg("-v").arg("error");
    if start_s > 0.0 { c.arg("-ss").arg(format!("{start_s:.3}")); }
    c.arg("-i").arg(path).args(["-vn", "-map", "0:a:0", "-ac", "1", "-ar"]).arg(rate.to_string())
        .args(["-af", "aresample=async=1:first_pts=0", "-f", "f32le", "-"]).stdout(Stdio::piped()).stderr(Stdio::null());
    c.spawn().with_context(|| format!("could not run FFmpeg on {}", path.display()))
}

/// Reads up to `max` samples from FFmpeg's output; empty at the end.
pub fn read_samples(out: &mut impl Read, max: usize) -> Result<Vec<f32>> {
    let mut buf = vec![0u8; max * 4];
    let mut got = 0;
    while got < buf.len() {
        let n = out.read(&mut buf[got..])?;
        if n == 0 { break; }
        got += n;
    }
    got -= got % 4;
    Ok(buf[..got].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

// ---------------------------------------------------------------------------------------------------------------------
// Downloading FFmpeg

/// A pinned FFmpeg build for this computer: a zip with the program in it.
#[derive(Debug, Clone, Serialize)]
pub struct Build { pub version: &'static str, pub urls: &'static [&'static str], pub sha256: &'static str, pub size: u64, pub member: &'static str, pub by: &'static str }

/// FFmpeg 9.1 builds. macOS and Linux: Martin Riedl's builds (the Linux one needs glibc 2.35 or newer, so Ubuntu 22.04,
/// Debian 12 or later); Windows: Gyan Doshi's "essentials" build (all GPL, with x264 for the review clips).
/// Never "latest": a new build means a new checksum here. Checksums match the .sha256 files the builders publish.
pub fn build() -> Option<Build> {
    const RIEDL: &str = "Martin Riedl (ffmpeg.martin-riedl.de)";
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(Build { version: "9.1", urls: &["https://ffmpeg.martin-riedl.de/download/macos/arm64/1795207104_9.1/ffmpeg.zip"],
            sha256: "1f66e9a690a7f1c9e60b012379f8869c11818d1797f45b21dbb2f2564ba61ae1", size: 28_623_086, member: "ffmpeg", by: RIEDL })
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some(Build { version: "9.1", urls: &["https://ffmpeg.martin-riedl.de/download/macos/amd64/1795212433_9.1/ffmpeg.zip"],
            sha256: "35b3d82cccc2c1d644d49d15f5e57eb913a08ef68f798042c90eecec62c51f1e", size: 34_064_287, member: "ffmpeg", by: RIEDL })
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(Build { version: "9.1", urls: &["https://ffmpeg.martin-riedl.de/download/linux/amd64/1795205506_9.1/ffmpeg.zip"],
            sha256: "707d4fa675932cc310e39e1506ac9224049786119689e345fed1c453a8a0896e", size: 33_573_944, member: "ffmpeg", by: RIEDL })
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some(Build { version: "9.1", urls: &["https://github.com/GyanD/codexffmpeg/releases/download/9.1/ffmpeg-9.1-essentials_build.zip",
            "https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-9.1-essentials_build.zip"],
            sha256: "b9bbbb3acece000cbeec994f856649354e52c773807841e78908374b24d658fd", size: 119_467_219, member: "ffmpeg-9.1-essentials_build/bin/ffmpeg.exe", by: "Gyan Doshi (gyan.dev)" })
    } else { None }
}

/// Downloads FFmpeg into `models_dir/ffmpeg`, checks the zip's SHA-256, and takes the program out of it.
pub fn install(models_dir: &Path, stop: &AtomicBool, progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
    let b = build().ok_or_else(|| anyhow!("There is no FFmpeg download for this computer. Install FFmpeg yourself (for example with Homebrew or apt) and Studio will find it."))?;
    install_from(&b, models_dir, stop, progress)
}

pub fn install_from(b: &Build, models_dir: &Path, stop: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
    let dir = ffmpeg_dir(models_dir);
    std::fs::create_dir_all(&dir)?;
    let zip_path = dir.join("ffmpeg.zip");
    let part = dir.join("ffmpeg.zip.part");
    let mut last = None;
    for url in b.urls {
        match crate::download::fetch(url, &part, b.size, stop, |n| progress(n, b.size)) {
            Ok(()) => { last = None; break; }
            Err(e) => { if stop.load(std::sync::atomic::Ordering::Relaxed) { bail!("Download stopped. It will continue where it left off."); } last = Some(e); }
        }
    }
    if let Some(e) = last { return Err(e).context("could not download FFmpeg"); }
    if crate::download::sha256_file(&part)? != b.sha256 {
        std::fs::remove_file(&part).ok();
        bail!("FFmpeg did not match its checksum, so it was not used. Please try again.");
    }
    std::fs::rename(&part, &zip_path)?;
    let mut z = zip::ZipArchive::new(std::fs::File::open(&zip_path)?)?;
    let dest = dir.join(EXE);
    {
        let mut f = z.by_name(b.member).context("the FFmpeg download has no program in it")?;
        let mut out = std::fs::File::create(dir.join(format!("{EXE}.part")))?;
        std::io::copy(&mut f, &mut out)?;
    }
    std::fs::rename(dir.join(format!("{EXE}.part")), &dest)?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?; }
    std::fs::write(dir.join("README.txt"), format!("FFmpeg {} built by {}, downloaded from {}\nFFmpeg is free software under the GNU GPL (this build): https://ffmpeg.org/legal.html\nSource code: https://ffmpeg.org/releases/ffmpeg-{}.tar.xz\n", b.version, b.by, b.urls[0], b.version))?;
    std::fs::remove_file(&zip_path).ok();
    progress(b.size, b.size);
    Ok(dest)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A video made by the tests themselves with FFmpeg (a test pattern and a 1 kHz tone), or None when FFmpeg is missing.
    pub fn make_video(dir: &Path, name: &str, secs: u32, sound: bool) -> Option<PathBuf> {
        ffmpeg()?;
        let out = dir.join(name);
        let mut c = command().unwrap();
        c.args(["-v", "error", "-y", "-f", "lavfi", "-i"]).arg(format!("testsrc2=size=320x240:rate=10:duration={secs}"));
        if sound { c.args(["-f", "lavfi", "-i"]).arg(format!("sine=frequency=1000:sample_rate=44100:duration={secs}")); }
        if name.ends_with(".avi") { c.args(["-c:v", "mjpeg"]); if sound { c.args(["-c:a", "pcm_s16le"]); } }
        c.args(["-metadata", "creation_time=2025-12-01T21:14:05Z"]).arg(&out);
        assert!(c.status().unwrap().success());
        Some(out)
    }

    /// A phone-style clip: HEVC tagged hvc1 in a .mov, optionally stored sideways with a 90 degree display rotation.
    /// None when FFmpeg or its HEVC encoder is missing.
    pub fn make_phone_video(dir: &Path, name: &str, rotated: bool) -> Option<PathBuf> {
        ffmpeg()?;
        let enc = command().unwrap().args(["-hide_banner", "-encoders"]).output().ok()?;
        if !String::from_utf8_lossy(&enc.stdout).contains("libx265") { return None; }
        let plain = dir.join(format!("{name}.plain.mov"));
        let st = command().unwrap().args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=10:duration=3",
            "-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000:duration=3",
            "-c:v", "libx265", "-tag:v", "hvc1", "-x265-params", "log-level=error", "-c:a", "aac",
            "-metadata", "creation_time=2025-12-01T21:14:05Z"]).arg(&plain).status().ok()?;
        assert!(st.success());
        let out = dir.join(name);
        if rotated {
            let st = command().unwrap().args(["-v", "error", "-y", "-display_rotation", "90", "-i"]).arg(&plain).args(["-c", "copy"]).arg(&out).status().ok()?;
            assert!(st.success());
        } else { std::fs::rename(&plain, &out).ok()?; }
        Some(out)
    }

    #[test]
    fn reads_the_rotation_phones_write() {
        let old = "  Stream #0:0[0x1](und): Video: hevc (Main) (hvc1 / 0x31637668), yuv420p(tv, bt709), 1920x1080, 9000 kb/s, 29.97 fps, 30 tbr, 600 tbn (default)
    Metadata:
      handler_name    : Core Media Video
    Side data:
      displaymatrix: rotation of -90.00 degrees
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, stereo, fltp, 160 kb/s (default)";
        assert_eq!((parse_probe(old).width, parse_probe(old).height), (Some(1080), Some(1920)));
        let new = old.replace("displaymatrix: rotation of -90.00", "Display Matrix: rotation of 90.00");
        assert_eq!((parse_probe(&new).width, parse_probe(&new).height), (Some(1080), Some(1920)));
        let upside = old.replace("-90.00", "180.00");
        assert_eq!((parse_probe(&upside).width, parse_probe(&upside).height), (Some(1920), Some(1080)));
        // A rotation on the sound stream (never seen, but harmless) does not swap the video.
        let none = old.replace("    Side data:\n      displaymatrix: rotation of -90.00 degrees\n", "");
        assert_eq!((parse_probe(&none).width, parse_probe(&none).height), (Some(1920), Some(1080)));
    }

    #[test]
    fn reads_what_ffmpeg_prints() {
        let text = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'badger.mp4':
  Metadata:
    major_brand     : isom
    creation_time   : 2025-12-01T21:14:05.000000Z
  Duration: 00:01:05.10, start: 0.000000, bitrate: 332 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt470bg/unknown/unknown, progressive), 640x480 [SAR 1:1 DAR 4:3], 324 kb/s, 29.97 fps, 30 tbr, 10240 tbn (default)
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, mono, fltp, 2 kb/s (default)
At least one output file must be specified";
        let v = parse_probe(text);
        assert_eq!((v.width, v.height, v.fps, v.audio_rate), (Some(640), Some(480), Some(29.97), Some(44100)));
        assert!((v.duration_s.unwrap() - 65.1).abs() < 1e-9);
        assert_eq!(v.created.unwrap().to_string(), "2025-12-01 21:14:05");
        // An AVI from a trail camera: MJPEG and PCM, no clock.
        let avi = "Input #0, avi, from 'IMAG0001.AVI':
  Duration: 00:00:10.00, start: 0.000000, bitrate: 33840 kb/s
  Stream #0:0: Video: mjpeg (Baseline) (MJPG / 0x47504A4D), yuvj420p(pc, bt470bg/unknown/unknown), 1280x720, 33061 kb/s, 30 fps, 30 tbr, 30 tbn
  Stream #0:1: Audio: pcm_s16le ([1][0][0][0] / 0x0001), 48000 Hz, mono, s16, 768 kb/s";
        let v = parse_probe(avi);
        assert_eq!((v.width, v.fps, v.audio_rate, v.created), (Some(1280), Some(30.0), Some(48000), None));
        // No sound; the MP4 epoch is no time.
        let v = parse_probe("  creation_time   : 1904-01-01T00:00:00.000000Z\n  Stream #0:0: Video: hevc (Main), yuv420p, 1920x1080, 25 fps, 25 tbr");
        assert_eq!((v.width, v.audio_rate, v.created), (Some(1920), None, None));
    }

    #[test]
    fn finds_the_sound_of_a_video() {
        let dir = tempfile::tempdir().unwrap();
        let Some(path) = make_video(dir.path(), "clip.mp4", 3, true) else { eprintln!("FFmpeg not installed: skipped"); return };
        let v = probe(&path).unwrap();
        assert_eq!((v.width, v.height, v.fps, v.audio_rate), (Some(320), Some(240), Some(10.0), Some(44100)));
        assert!((v.duration_s.unwrap() - 3.0).abs() < 0.1);
        let mut child = sound(&path, 48_000, 1.0).unwrap();
        let s = read_samples(child.stdout.as_mut().unwrap(), 10 * 48_000).unwrap();
        let _ = child.wait();
        assert!((s.len() as f64 / 48_000.0 - 2.0).abs() < 0.05, "{} samples", s.len());
        assert!(s.iter().fold(0f32, |m, &x| m.max(x.abs())) > 0.1);
    }
}
