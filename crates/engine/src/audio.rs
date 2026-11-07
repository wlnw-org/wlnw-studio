//! Streaming audio reader: any WAV, FLAC or MP3, mixed to mono and resampled to 48 kHz, handed out in 3 s windows.
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

pub struct AudioStream {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
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

/// Opens a recording, resampled to `rate` (48 kHz for BirdNET, 32 kHz for Perch).
pub fn open(path: &Path, rate: u32) -> Result<AudioStream> {
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
    let decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let resampler = if source_rate == rate { None } else { Some(FftFixedIn::<f32>::new(source_rate as usize, rate as usize, 4096, 2, 1)?) };
    Ok(AudioStream { track: track.id, format, decoder, source_rate, rate, resampler, pending: Vec::new(), out: Vec::new(), eof: false, position: 0 })
}

impl AudioStream {
    /// Decodes one packet into `pending` (mono, source rate). Returns false at the end of the file.
    fn decode_packet(&mut self) -> Result<bool> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(SymError::ResetRequired) => return Ok(false),
                Err(e) => return Err(e.into()),
            };
            if packet.track_id() != self.track { continue; }
            let decoded = match self.decoder.decode(&packet) {
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

/// Reads `secs` seconds starting at `start_s` (48 kHz mono), for review clips.
pub fn read_segment(path: &Path, start_s: f64, secs: f64) -> Result<Vec<f32>> {
    use symphonia::core::formats::{SeekMode, SeekTo};
    use symphonia::core::units::Time;
    let mut s = open(path, SR)?;
    // Jump close to the clip instead of decoding the whole recording up to it; fall back to decoding when a format cannot seek.
    let mut at = 0f64;
    if start_s > 0.0 {
        let t = Time { seconds: start_s.floor() as u64, frac: start_s.fract() };
        if let Ok(done) = s.format.seek(SeekMode::Accurate, SeekTo::Time { time: t, track_id: Some(s.track) }) {
            s.decoder.reset();
            at = done.actual_ts as f64 / s.source_rate as f64;
        }
    }
    let skip = ((start_s - at).max(0.0) * SR as f64) as usize;
    let want = (secs * SR as f64) as usize;
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
