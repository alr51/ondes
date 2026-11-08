//! Aperçu des effets (développement) : une image PPM par effet.
//!
//!     cargo run -p ondes-visual --example preview -- <dossier>

use ondes_visual::{BANDS, EFFECTS, Inputs, Renderer};

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let mut renderer = Renderer::new().expect("carte graphique");
    // Spectre de musique typique : graves forts, aigus qui décroissent.
    let mut inputs = Inputs {
        time: 7.,
        level: 0.55,
        beat: 0.6,
        bass: 0.8,
        mid: 0.5,
        treble: 0.3,
        ..Default::default()
    };
    for (i, b) in inputs.bands.iter_mut().enumerate() {
        let x = i as f32 / BANDS as f32;
        *b = (0.9 - 0.7 * x + 0.15 * (x * 25.).sin()).clamp(0., 1.);
    }
    let (w, h) = (640, 360);
    for (i, (name, _)) in EFFECTS.iter().enumerate() {
        let bgra = renderer.render(i, w, h, &inputs).expect("rendu");
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        for px in bgra.as_chunks::<4>().0 {
            ppm.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        let path = dir.join(format!("effet-{i}.ppm"));
        std::fs::write(&path, ppm).expect("écriture");
        println!("{name} → {}", path.display());
    }
}
