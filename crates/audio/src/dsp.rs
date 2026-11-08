//! Traitement appliqué à la lecture : égaliseur, gain lissé, prise
//! d'échantillons pour le spectre.
//!
//! Tout se fait dans [`crate::source::StreamingSource`], au moment où les
//! échantillons partent vers la sortie : un réglage s'entend aussitôt, même
//! avec plusieurs secondes d'audio décodé d'avance.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// Fréquences centrales des 10 bandes (octaves ISO).
pub const EQ_FREQS: [f32; 10] = [
    31., 62., 125., 250., 500., 1_000., 2_000., 4_000., 8_000., 16_000.,
];
/// Correction maximale d'une bande, en dB (dans les deux sens).
pub const EQ_MAX_DB: f32 = 12.;
/// Facteur de qualité : une octave de large.
const EQ_Q: f32 = 1.41;

/// Réglages de l'égaliseur.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Equalizer {
    pub enabled: bool,
    /// Correction de chaque bande de [`EQ_FREQS`], en dB.
    pub gains: [f32; 10],
}

impl Equalizer {
    fn is_flat(&self) -> bool {
        !self.enabled || self.gains.iter().all(|g| g.abs() < 0.05)
    }
}

/// État partagé entre le moteur et les sources en lecture.
#[derive(Default)]
pub(crate) struct DspShared {
    eq: Mutex<Equalizer>,
    /// Incrémenté à chaque réglage : les sources recalculent leurs filtres.
    eq_version: AtomicU64,
    pub(crate) tap: Tap,
}

impl DspShared {
    pub(crate) fn set_equalizer(&self, eq: Equalizer) {
        let mut eq = eq;
        for g in &mut eq.gains {
            *g = g.clamp(-EQ_MAX_DB, EQ_MAX_DB);
        }
        *self.eq.lock().unwrap_or_else(|e| e.into_inner()) = eq;
        self.eq_version.fetch_add(1, Ordering::Release);
    }

    fn equalizer(&self) -> Equalizer {
        *self.eq.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Filtre biquad (forme transposée II), coefficients normalisés.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    /// Filtre en cloche (« Audio EQ Cookbook », R. Bristow-Johnson).
    fn peaking(freq: f32, gain_db: f32, rate: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.);
        let w0 = std::f32::consts::TAU * freq / rate;
        let alpha = w0.sin() / (2. * EQ_Q);
        let cos = w0.cos();
        let a0 = 1. + alpha / a;
        Self {
            b0: (1. + alpha * a) / a0,
            b1: -2. * cos / a0,
            b2: (1. - alpha * a) / a0,
            a1: -2. * cos / a0,
            a2: (1. - alpha / a) / a0,
        }
    }
}

/// Égaliseur d'une source : filtres et mémoire de chaque canal.
pub(crate) struct EqProcessor {
    version: u64,
    channels: usize,
    rate: f32,
    filters: Vec<Biquad>,
    /// `[z1, z2]` par filtre et par canal.
    state: Vec<[f32; 2]>,
    /// Atténuation qui compense la bande la plus amplifiée (pas d'écrêtage).
    preamp: f32,
}

impl EqProcessor {
    pub(crate) fn new(channels: usize, rate: u32) -> Self {
        Self {
            version: u64::MAX,
            channels: channels.max(1),
            rate: rate as f32,
            filters: Vec::new(),
            state: Vec::new(),
            preamp: 1.,
        }
    }

    fn configure(&mut self, eq: &Equalizer) {
        self.filters.clear();
        if !eq.is_flat() {
            for (&freq, &gain) in EQ_FREQS.iter().zip(&eq.gains) {
                // Bandes neutres ou au-delà de Nyquist : rien à faire.
                if gain.abs() >= 0.05 && freq < self.rate * 0.45 {
                    self.filters.push(Biquad::peaking(freq, gain, self.rate));
                }
            }
        }
        let boost = eq.gains.iter().copied().fold(0f32, f32::max);
        self.preamp = if self.filters.is_empty() {
            1.
        } else {
            10f32.powf(-boost / 20.)
        };
        // Mémoire gardée si possible : pas de clic au changement de réglage.
        self.state
            .resize(self.filters.len() * self.channels, [0., 0.]);
    }

    /// Filtre un bloc d'échantillons entrelacés.
    pub(crate) fn process(&mut self, samples: &mut [f32], dsp: &DspShared) {
        let version = dsp.eq_version.load(Ordering::Acquire);
        if version != self.version {
            self.version = version;
            self.configure(&dsp.equalizer());
        }
        if self.filters.is_empty() {
            return;
        }
        let n = self.filters.len();
        for frame in samples.chunks_mut(self.channels) {
            for (ch, sample) in frame.iter_mut().enumerate() {
                let mut x = *sample * self.preamp;
                for (f, z) in self
                    .filters
                    .iter()
                    .zip(&mut self.state[ch * n..(ch + 1) * n])
                {
                    let y = f.b0 * x + z[0];
                    z[0] = f.b1 * x - f.a1 * y + z[1];
                    z[1] = f.b2 * x - f.a2 * y;
                    x = y;
                }
                *sample = x;
            }
        }
    }
}

/// Gain appliqué échantillon par échantillon, qui rejoint sa cible en douceur
/// (volume, ReplayGain, fondus) : pas de saut audible.
pub(crate) struct SmoothGain {
    current: Option<f32>,
    coef: f32,
}

impl SmoothGain {
    /// Constante de temps du lissage.
    const TAU_SECS: f32 = 0.04;

    pub(crate) fn new(rate: u32) -> Self {
        Self {
            current: None,
            coef: 1. - (-1. / (Self::TAU_SECS * rate as f32)).exp(),
        }
    }

    /// Gain pour la trame suivante.
    pub(crate) fn next(&mut self, target: f32) -> f32 {
        let g = match self.current {
            // Premier échantillon : directement à la cible.
            None => target,
            Some(c) if (c - target).abs() < 1e-4 => target,
            Some(c) => c + (target - c) * self.coef,
        };
        self.current = Some(g);
        g
    }
}

/// Taille de la fenêtre d'analyse (puissance de 2).
pub const FFT_SIZE: usize = 2048;

/// Derniers échantillons joués (mono), pour le spectre.
#[derive(Default)]
pub(crate) struct Tap {
    enabled: AtomicBool,
    /// Visualiseurs ouverts : la prise reste active même si le petit
    /// spectre est masqué.
    holders: AtomicUsize,
    ring: Mutex<Ring>,
}

#[derive(Default)]
struct Ring {
    buf: Vec<f32>,
    pos: usize,
    rate: u32,
}

impl Tap {
    pub(crate) fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed) || self.holders.load(Ordering::Relaxed) > 0
    }

    /// Ajoute un bloc entrelacé, réduit en mono.
    pub(crate) fn push(&self, samples: &[f32], channels: usize, rate: u32) {
        let Ok(mut ring) = self.ring.lock() else {
            return;
        };
        if ring.buf.len() != FFT_SIZE {
            ring.buf = vec![0.; FFT_SIZE];
            ring.pos = 0;
        }
        ring.rate = rate;
        let channels = channels.max(1);
        let scale = 1. / channels as f32;
        for frame in samples.chunks(channels) {
            let pos = ring.pos;
            ring.buf[pos] = frame.iter().sum::<f32>() * scale;
            ring.pos = (pos + 1) % FFT_SIZE;
        }
    }
}

/// Analyseur de spectre, utilisable depuis l'UI.
#[derive(Clone)]
pub struct Analyzer(pub(crate) std::sync::Arc<DspShared>);

impl Analyzer {
    /// Active ou coupe la prise d'échantillons (économise le calcul).
    pub fn set_enabled(&self, on: bool) {
        self.0.tap.enabled.store(on, Ordering::Relaxed);
        if !on && let Ok(mut ring) = self.0.tap.ring.lock() {
            ring.buf.iter_mut().for_each(|s| *s = 0.);
        }
    }

    /// Garde la prise d'échantillons active tant que le jeton existe.
    pub fn hold(&self) -> AnalyzerHold {
        self.0.tap.holders.fetch_add(1, Ordering::Relaxed);
        AnalyzerHold(self.0.clone())
    }

    /// Niveau (0 à 1) de `bars.len()` bandes réparties en échelle
    /// logarithmique de 40 Hz à 16 kHz.
    pub fn spectrum(&self, bars: &mut [f32]) {
        bars.iter_mut().for_each(|b| *b = 0.);
        let (mut re, rate) = {
            let Ok(ring) = self.0.tap.ring.lock() else {
                return;
            };
            if ring.buf.len() != FFT_SIZE || ring.rate == 0 {
                return;
            }
            // Ordre chronologique, fenêtre de Hann.
            let re: Vec<f32> = (0..FFT_SIZE)
                .map(|i| {
                    let w = 0.5
                        - 0.5 * (std::f32::consts::TAU * i as f32 / (FFT_SIZE - 1) as f32).cos();
                    ring.buf[(ring.pos + i) % FFT_SIZE] * w
                })
                .collect();
            (re, ring.rate as f32)
        };
        let mut im = vec![0f32; FFT_SIZE];
        fft(&mut re, &mut im);

        let n = bars.len();
        if n == 0 {
            return;
        }
        let bin_hz = rate / FFT_SIZE as f32;
        let (lo, hi) = (40f32, 16_000f32.min(rate / 2.));
        for (i, bar) in bars.iter_mut().enumerate() {
            let f0 = lo * (hi / lo).powf(i as f32 / n as f32);
            let f1 = lo * (hi / lo).powf((i + 1) as f32 / n as f32);
            let b0 = ((f0 / bin_hz) as usize).max(1);
            let b1 = ((f1 / bin_hz) as usize).clamp(b0 + 1, FFT_SIZE / 2);
            let power = (b0..b1)
                .map(|k| re[k] * re[k] + im[k] * im[k])
                .fold(0f32, f32::max);
            // Amplitude normalisée (fenêtre de Hann : gain 1/2), en dB.
            let amp = power.sqrt() * 4. / FFT_SIZE as f32;
            let db = 20. * amp.max(1e-9).log10();
            // -60 dB → 0, 0 dB → 1.
            *bar = ((db + 60.) / 60.).clamp(0., 1.);
        }
    }
}

/// Jeton de [`Analyzer::hold`].
pub struct AnalyzerHold(std::sync::Arc<DspShared>);

impl Drop for AnalyzerHold {
    fn drop(&mut self) {
        self.0.tap.holders.fetch_sub(1, Ordering::Relaxed);
    }
}

/// FFT radix-2 en place (taille : puissance de 2).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    // Permutation par inversion des bits.
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -std::f32::consts::TAU / len as f32;
        let (w_im, w_re) = angle.sin_cos();
        for start in (0..n).step_by(len) {
            let (mut cur_re, mut cur_im) = (1f32, 0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let t_re = re[b] * cur_re - im[b] * cur_im;
                let t_im = re[b] * cur_im + im[b] * cur_re;
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
                let next_re = cur_re * w_re - cur_im * w_im;
                cur_im = cur_re * w_im + cur_im * w_re;
                cur_re = next_re;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn sine(freq: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[test]
    fn fft_finds_a_sine() {
        let mut re = sine(48_000. / 2048. * 100., 48_000, 2048); // pile sur le bin 100
        let mut im = vec![0.; 2048];
        fft(&mut re, &mut im);
        let peak = (1..1024)
            .max_by(|&a, &b| {
                (re[a].hypot(im[a]))
                    .partial_cmp(&re[b].hypot(im[b]))
                    .unwrap()
            })
            .unwrap();
        assert_eq!(peak, 100);
    }

    #[test]
    fn equalizer_boosts_and_cuts_its_band() {
        let dsp = DspShared::default();
        let mut gains = [0.; 10];
        gains[5] = 12.; // 1 kHz
        gains[1] = -12.; // 62 Hz
        dsp.set_equalizer(Equalizer {
            enabled: true,
            gains,
        });
        let level = |freq: f32| {
            let mut eq = EqProcessor::new(1, 48_000);
            let mut s = sine(freq, 48_000, 48_000);
            eq.process(&mut s, &dsp);
            // Après le régime transitoire.
            20. * (rms(&s[24_000..]) / rms(&sine(freq, 48_000, 24_000))).log10()
        };
        // Préampli -12 dB : la bande amplifiée revient à 0, les autres baissent.
        assert!(level(1_000.).abs() < 0.5, "{}", level(1_000.));
        assert!((level(62.) + 24.).abs() < 1., "{}", level(62.));
        assert!((level(16_000.) + 12.).abs() < 1., "{}", level(16_000.));

        // Désactivé : signal intact.
        dsp.set_equalizer(Equalizer {
            enabled: false,
            gains,
        });
        let mut eq = EqProcessor::new(2, 48_000);
        let original = sine(440., 48_000, 1_000);
        let mut s = original.clone();
        eq.process(&mut s, &dsp);
        assert_eq!(s, original);
    }

    #[test]
    fn gain_ramps_smoothly() {
        let mut g = SmoothGain::new(48_000);
        assert_eq!(g.next(1.), 1.);
        let after_10ms: Vec<f32> = (0..480).map(|_| g.next(0.)).collect();
        assert!(after_10ms.windows(2).all(|w| w[1] < w[0]));
        assert!(after_10ms[479] > 0.5 && after_10ms[479] < 0.9);
        for _ in 0..48_000 {
            g.next(0.);
        }
        assert_eq!(g.next(0.), 0.);
    }

    #[test]
    fn spectrum_peaks_at_the_played_frequency() {
        let analyzer = Analyzer(Arc::new(DspShared::default()));
        analyzer.set_enabled(true);
        // Stéréo, 1 kHz.
        let mono = sine(1_000., 48_000, 4096);
        let stereo: Vec<f32> = mono.iter().flat_map(|&s| [s, s]).collect();
        analyzer.0.tap.push(&stereo, 2, 48_000);
        let mut bars = [0f32; 24];
        analyzer.spectrum(&mut bars);
        let loudest = (0..24)
            .max_by(|&a, &b| bars[a].partial_cmp(&bars[b]).unwrap())
            .unwrap();
        // 1 kHz sur une échelle log 40 Hz – 16 kHz : vers 54 % de la largeur.
        assert!((12..=14).contains(&loudest), "{loudest} {bars:?}");
        assert!(bars[loudest] > 0.8);
        assert!(bars[0] < 0.3);
    }
}
