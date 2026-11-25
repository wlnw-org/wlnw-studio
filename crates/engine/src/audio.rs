//! Streaming audio reader: any WAV, FLAC, MP3 or AAC (.m4a, .aac), or the sound of a video (through FFmpeg), mixed to mono and resampled to 48 kHz, handed out in 3 s windows.
//! Memory stays small however long the file is.
use crate::SR;
use anyhow::{anyhow, Context, Result};
use rubato::{FftFixedIn, Resampler};
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Where the samples come from: Symphonia for sound files, FFmpeg for the sound track of a video.
enum Src {
    Sym {
        format: Box<dyn FormatReader>,
        decoder: Box<dyn Decoder>,
        track: u32,
        /// The track's timestamp unit (MP4 tracks may count in something other than samples).
        time_base: Option<symphonia::core::units::TimeBase>,
    },
    Ff { child: std::process::Child },
}

pub struct AudioStream {
    src: Src,
    pub source_rate: u32,
    /// The rate windows are handed out at.
    pub rate: u32,
    resampler: Option<FftFixedIn<f32>>,
    pending: Vec<f32>,
    out: Vec<f32>,
    eof: bool,
    /// Samples (at `rate`) already handed out, so callers know where each window starts.
    pub position: usize,
}

/// The sample rate written in a recording's header, without decoding any audio.
pub fn probe_rate(path: &Path) -> Option<u32> {
    if crate::video::is_video(path) { return crate::video::probe(path).ok()?.audio_rate; }
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) { hint.with_extension(ext); }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    probed.format.default_track()?.codec_params.sample_rate
}

/// A recording's length from its header (frames / rate), without decoding; None when the header does not say (some MP3s).
pub fn duration(path: &Path) -> Option<f64> {
    if crate::video::is_video(path) { return crate::video::probe(path).ok()?.duration_s; }
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) { hint.with_extension(ext); }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    let c = &probed.format.default_track()?.codec_params;
    Some(c.n_frames? as f64 / c.sample_rate? as f64)
}

/// The rate a recording was made at: the header rate times the time expansion factor GUANO notes for time-expanded
/// bat recordings (a 10x recording written at 38.4 kHz was made at 384 kHz).
pub fn effective_rate(path: &Path) -> Option<u32> {
    let te = crate::meta::read(path).time_expansion.unwrap_or(1.0);
    probe_rate(path).map(|r| (r as f64 * te).round() as u32)
}

/// Opens a recording at its own sample rate (no resampling), mixed to mono.
pub fn open_native(path: &Path) -> Result<AudioStream> {
    let rate = probe_rate(path).ok_or_else(|| anyhow!("not an audio file we can read: {}", path.display()))?;
    open(path, rate)
}

/// Opens a recording, resampled to `rate` (48 kHz for BirdNET, 32 kHz for Perch).
pub fn open(path: &Path, rate: u32) -> Result<AudioStream> {
    if crate::video::is_video(path) { return open_video(path, rate, 0.0); }
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) { hint.with_extension(ext); }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .with_context(|| format!("not an audio file we can read: {}", path.display()))?;
    let format = probed.format;
    let track = format.default_track().ok_or_else(|| anyhow!("no audio track"))?;
    let source_rate = track.codec_params.sample_rate.ok_or_else(|| anyhow!("unknown sample rate"))?;
    // A damaged or crafted header could ask for an absurd rate, and the resampler would size its buffers from it.
    if !(1_000..=768_000).contains(&source_rate) { return Err(anyhow!("unsupported sample rate {source_rate} Hz")); }
    let decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let resampler = if source_rate == rate { None } else { Some(FftFixedIn::<f32>::new(source_rate as usize, rate as usize, 4096, 2, 1)?) };
    Ok(AudioStream { src: Src::Sym { track: track.id, time_base: track.codec_params.time_base, format, decoder }, source_rate, rate, resampler, pending: Vec::new(), out: Vec::new(), eof: false, position: 0 })
}

/// A video's sound track from `start_s`, decoded and resampled to `rate` by FFmpeg.
fn open_video(path: &Path, rate: u32, start_s: f64) -> Result<AudioStream> {
    let child = crate::video::sound(path, rate, start_s)?;
    Ok(AudioStream { src: Src::Ff { child }, source_rate: rate, rate, resampler: None, pending: Vec::new(), out: Vec::new(), eof: false, position: 0 })
}

impl Drop for AudioStream {
    fn drop(&mut self) {
        if let Src::Ff { child } = &mut self.src { let _ = child.kill(); let _ = child.wait(); }
    }
}

impl AudioStream {
    /// Decodes one packet into `pending` (mono, source rate). Returns false at the end of the file.
    fn decode_packet(&mut self) -> Result<bool> {
        let (format, decoder, track) = match &mut self.src {
            Src::Sym { format, decoder, track, .. } => (format, decoder, *track),
            Src::Ff { child } => {
                let got = crate::video::read_samples(child.stdout.as_mut().ok_or_else(|| anyhow!("FFmpeg gave no output"))?, 16_384)?;
                if got.is_empty() { return Ok(false); }
                self.pending.extend_from_slice(&got);
                return Ok(true);
            }
        };
        loop {
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(SymError::ResetRequired) => return Ok(false),
                Err(e) => return Err(e.into()),
            };
            if packet.track_id() != track { continue; }
            let decoded = match decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymError::DecodeError(_)) => continue, // a damaged packet: skip it, as recorders sometimes write one
                Err(e) => return Err(e.into()),
            };
            let spec = *decoded.spec();
            let ch = spec.channels.count().max(1);
            let mut sb = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            sb.copy_interleaved_ref(decoded);
            for frame in sb.samples().chunks(ch) { self.pending.push(frame.iter().sum::<f32>() / ch as f32); }
            return Ok(true);
        }
    }

    fn fill(&mut self, want: usize) -> Result<()> {
        while self.out.len() < want && !self.eof {
            if !self.decode_packet()? { self.eof = true; }
            match self.resampler.as_mut() {
                None => self.out.append(&mut self.pending),
                Some(r) => {
                    loop {
                        let need = r.input_frames_next();
                        if self.pending.len() >= need {
                            let rest = self.pending.split_off(need);
                            let res = r.process(&[std::mem::replace(&mut self.pending, rest)], None)?;
                            self.out.extend_from_slice(&res[0]);
                        } else if self.eof && !self.pending.is_empty() {
                            let res = r.process_partial(Some(&[std::mem::take(&mut self.pending)]), None)?;
                            self.out.extend_from_slice(&res[0]);
                        } else { break; }
                    }
                }
            }
        }
        Ok(())
    }

    /// The next window of `window` samples, padded with silence at the end of the file (a last piece shorter than `min_len` is dropped); None when done.
    pub fn next_window(&mut self, window: usize, min_len: usize) -> Result<Option<(usize, Vec<f32>)>> {
        self.fill(window)?;
        if self.out.is_empty() || (self.out.len() < window && self.out.len() < min_len) { return Ok(None); }
        let take = self.out.len().min(window);
        let mut c: Vec<f32> = self.out.drain(..take).collect();
        c.resize(window, 0.0);
        let start = self.position;
        self.position += take;
        Ok(Some((start, c)))
    }
}

impl AudioStream {
    /// Samples decoded but not handed out (after `next_window` returned None: the piece too short for a window).
    pub fn leftover(&self) -> usize { self.out.len() }
}

/// Reads `secs` seconds starting at `start_s` (48 kHz mono), for review clips.
pub fn read_segment(path: &Path, start_s: f64, secs: f64) -> Result<Vec<f32>> {
    // FFmpeg seeks itself.
    if crate::video::is_video(path) { return segment(open_video(path, SR, start_s)?, 0.0, secs); }
    segment(open(path, SR)?, start_s, secs)
}

/// Like `read_segment`, at the recording's own sample rate (for ultrasonic bat clips). Returns the samples and that rate.
pub fn read_segment_native(path: &Path, start_s: f64, secs: f64) -> Result<(Vec<f32>, u32)> {
    if crate::video::is_video(path) {
        let rate = probe_rate(path).ok_or_else(|| anyhow!("this video has no sound"))?;
        return Ok((segment(open_video(path, rate, start_s)?, 0.0, secs)?, rate));
    }
    let s = open_native(path)?;
    let rate = s.rate;
    Ok((segment(s, start_s, secs)?, rate))
}

fn segment(mut s: AudioStream, start_s: f64, secs: f64) -> Result<Vec<f32>> {
    use symphonia::core::formats::{SeekMode, SeekTo};
    use symphonia::core::units::Time;
    let rate = s.rate as f64;
    // Jump close to the clip instead of decoding the whole recording up to it; fall back to decoding when a format cannot seek.
    let mut at = 0f64;
    if start_s > 0.0 {
        let t = Time { seconds: start_s.floor() as u64, frac: start_s.fract() };
        if let Src::Sym { format, decoder, track, time_base } = &mut s.src {
            if let Ok(done) = format.seek(SeekMode::Accurate, SeekTo::Time { time: t, track_id: Some(*track) }) {
                decoder.reset();
                at = match time_base { Some(tb) => { let t = tb.calc_time(done.actual_ts); t.seconds as f64 + t.frac }, None => done.actual_ts as f64 / s.source_rate as f64 };
            }
        }
    }
    let skip = ((start_s - at).max(0.0) * rate) as usize;
    let want = (secs * rate) as usize;
    let mut out = Vec::with_capacity(want);
    let mut skipped = 0usize;
    while skipped < skip {
        s.fill(1)?;
        if s.out.is_empty() { break; }
        let n = (skip - skipped).min(s.out.len());
        s.out.drain(..n);
        skipped += n;
    }
    while out.len() < want {
        s.fill(want - out.len())?;
        if s.out.is_empty() { break; }
        let n = (want - out.len()).min(s.out.len());
        out.extend(s.out.drain(..n));
    }
    Ok(out)
}

/// 16-bit PCM WAV bytes, for playing a clip in the app.
pub fn wav_bytes(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    let peak = samples.iter().fold(0f32, |m, &v| m.max(v.abs())).max(1e-6);
    let gain = (0.9 / peak).min(20.0); // quiet clips are made audible, never more than 20x
    for &v in samples { b.extend_from_slice(&((((v * gain).clamp(-1.0, 1.0)) * 32767.0).round() as i16).to_le_bytes()); }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1.5 s, 3 kHz tone, AAC in .m4a at 48 kHz mono: the format a Wildlife Network Base (BirdNET-Go) saves clips in.
    fn m4a() -> std::path::PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/tone.m4a") }

    #[test]
    fn decodes_aac_m4a() {
        let p = m4a();
        assert_eq!(probe_rate(&p), Some(48_000));
        let file = std::fs::File::open(&p).unwrap();
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut format = symphonia::default::get_probe().format(Hint::new().with_extension("m4a"), mss, &FormatOptions::default(), &MetadataOptions::default()).unwrap().format;
        // MP4 headers do not always state the channels for AAC; the decoded audio does.
        let mut dec = symphonia::default::get_codecs().make(&format.default_track().unwrap().codec_params, &DecoderOptions::default()).unwrap();
        let packet = format.next_packet().unwrap();
        let spec = *dec.decode(&packet).unwrap().spec();
        assert_eq!((spec.channels.count(), spec.rate), (1, 48_000));
        let all = read_segment(&p, 0.0, 10.0).unwrap();
        let secs = all.len() as f64 / SR as f64;
        assert!((secs - 1.5).abs() < 0.1, "decoded {secs} s");
        if let Some(d) = duration(&p) { assert!((d - 1.5).abs() < 0.1, "header says {d} s"); }
        // Real sound, not silence.
        assert!(all.iter().fold(0f32, |m, &v| m.max(v.abs())) > 0.1);
    }

    #[test]
    fn seeks_in_aac_m4a() {
        let p = m4a();
        let part = read_segment(&p, 0.5, 0.5).unwrap();
        assert_eq!(part.len(), SR as usize / 2);
        let whole = read_segment(&p, 0.0, 10.0).unwrap();
        // The seek lands on the same audio as decoding from the start (within one AAC frame of level).
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        let from_start = &whole[SR as usize / 2..SR as usize];
        assert!((rms(&part) - rms(from_start)).abs() < 0.05);
    }
}
