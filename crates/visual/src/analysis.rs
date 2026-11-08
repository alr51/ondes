//! Du spectre brut aux paramètres des shaders : lissage, niveaux par
//! registre, détection des temps forts, horloge musicale.

use crate::BANDS;

/// Paramètres transmis aux shaders (voir `shaders/prelude.wgsl`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inputs {
    pub time: f32,
    pub level: f32,
    pub beat: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub bands: [f32; BANDS],
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            time: 0.,
            level: 0.,
            beat: 0.,
            bass: 0.,
            mid: 0.,
            treble: 0.,
            bands: [0.; BANDS],
        }
    }
}

#[derive(Debug, Default)]
pub struct Analysis {
    out: Inputs,
    /// Moyenne glissante de l'énergie des graves (référence des temps forts).
    bass_avg: f32,
    since_beat: f32,
}

/// Retombée d'une bande par seconde.
const FALL_PER_SEC: f32 = 1.6;
/// Écart minimal entre deux temps forts (≈ 270 BPM).
const MIN_BEAT_GAP: f32 = 0.22;

fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len().max(1) as f32
}

impl Analysis {
    /// `raw` : niveaux bruts (0 à 1) ; `dt` : secondes depuis l'appel précédent.
    pub fn update(&mut self, raw: &[f32; BANDS], dt: f32) -> Inputs {
        // Le spectre musical tourne autour de 0,5 : on étire la plage utile.
        let expand = |v: f32| ((v - 0.3) / 0.6).clamp(0., 1.);
        for (s, &r) in self.out.bands.iter_mut().zip(raw) {
            *s = expand(r).max(*s - FALL_PER_SEC * dt);
        }
        let b = &self.out.bands;
        self.out.level = mean(b);
        self.out.bass = mean(&b[..10]);
        self.out.mid = mean(&b[10..36]);
        self.out.treble = mean(&b[36..]);

        // Temps fort : les graves dépassent nettement leur moyenne récente.
        let bass_now = mean(&raw[..8].iter().map(|&v| expand(v)).collect::<Vec<_>>());
        self.since_beat += dt;
        if bass_now > self.bass_avg * 1.25 + 0.05 && self.since_beat > MIN_BEAT_GAP {
            self.out.beat = 1.;
            self.since_beat = 0.;
        } else {
            self.out.beat *= (-dt * 5.).exp();
        }
        self.bass_avg += (bass_now - self.bass_avg) * (dt * 2.).min(1.);

        // Le temps s'accélère avec le volume : les effets vivent avec la musique.
        self.out.time += dt * (0.4 + 1.2 * self.out.level);
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_beats_and_falls_back() {
        let mut a = Analysis::default();
        let quiet = [0.3; BANDS];
        let mut kick = [0.3; BANDS];
        kick[..8].fill(0.95);
        for _ in 0..30 {
            a.update(&quiet, 0.033);
        }
        let out = a.update(&kick, 0.033);
        assert_eq!(out.beat, 1.);
        assert!(out.bass > out.treble);
        // Pas de second temps fort tout de suite.
        let out = a.update(&kick, 0.033);
        assert!(out.beat < 1.);
        // Silence : tout retombe.
        let silent = [0.; BANDS];
        let mut out = out;
        for _ in 0..60 {
            out = a.update(&silent, 0.033);
        }
        assert_eq!(out.level, 0.);
        assert!(out.beat < 0.01);
        assert!(out.time > 0.);
    }
}
