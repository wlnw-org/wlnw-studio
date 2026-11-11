//! scipy.signal.resample_poly (SciPy, BSD 3-Clause licence, https://scipy.org), which BatDetect2 uses to bring every
//! clip to 256 kHz, translated to Rust so Studio's resampling matches BatDetect2's.
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
