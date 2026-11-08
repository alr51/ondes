//! Visualiseurs d'Ondes : effets écrits en WGSL, animés par le spectre.
//!
//! GPUI ne permet pas d'insérer ses propres shaders dans son rendu : chaque
//! image est donc rendue hors écran avec `wgpu`, sur un thread dédié, puis
//! relue (BGRA) pour être affichée comme une image ordinaire. À 30 images
//! par seconde et à résolution réduite, la copie reste négligeable.

mod analysis;
mod render;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use analysis::{Analysis, Inputs};
pub use render::Renderer;

/// Nombre de bandes du spectre transmises aux shaders.
pub const BANDS: usize = 64;

/// Effets disponibles : nom affiché et source WGSL.
pub const EFFECTS: &[(&str, &str)] = &[
    ("Nébuleuse", include_str!("shaders/nebula.wgsl")),
    ("Anneau", include_str!("shaders/ring.wgsl")),
    ("Tunnel", include_str!("shaders/tunnel.wgsl")),
    ("Ondes", include_str!("shaders/waves.wgsl")),
];

const PRELUDE: &str = include_str!("shaders/prelude.wgsl");
/// Cadence visée.
const FRAME_TIME: Duration = Duration::from_millis(33);
/// Largeur maximale du rendu (l'image est agrandie à l'affichage).
pub const MAX_WIDTH: u32 = 800;

#[derive(Debug, thiserror::Error)]
pub enum VisualError {
    #[error("aucune carte graphique utilisable : {0}")]
    NoAdapter(String),
    #[error("carte graphique indisponible : {0}")]
    Device(String),
    #[error("rendu impossible : {0}")]
    Render(String),
}

/// Image rendue, en BGRA 8 bits sans marge.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// Numéro croissant : une nouvelle image a un numéro plus grand.
    pub seq: u64,
}

/// Source du spectre : remplit `BANDS` niveaux de 0 à 1.
pub type SpectrumFn = Box<dyn FnMut(&mut [f32]) + Send>;

#[derive(Debug, Clone, Copy)]
struct Control {
    effect: usize,
    width: u32,
    height: u32,
    playing: bool,
}

#[derive(Default)]
struct Output {
    frame: Option<Frame>,
    error: Option<String>,
}

/// Rendu en continu sur un thread dédié ; l'arrêter en l'abandonnant.
pub struct Visualizer {
    control: Arc<Mutex<Control>>,
    output: Arc<Mutex<Output>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Visualizer {
    pub fn start(effect: usize, spectrum: SpectrumFn) -> Self {
        let control = Arc::new(Mutex::new(Control {
            effect,
            width: 0,
            height: 0,
            playing: false,
        }));
        let output = Arc::new(Mutex::new(Output::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (control, output, stop) = (control.clone(), output.clone(), stop.clone());
            std::thread::Builder::new()
                .name("ondes-visual".into())
                .spawn(move || run(control, output, stop, spectrum))
                .map_err(|e| tracing::error!("thread du visualiseur : {e}"))
                .ok()
        };
        Self {
            control,
            output,
            stop,
            thread,
        }
    }

    fn control(&self) -> std::sync::MutexGuard<'_, Control> {
        self.control.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_effect(&self, effect: usize) {
        self.control().effect = effect % EFFECTS.len();
    }

    /// Taille d'affichage, en pixels ; le rendu est réduit à [`MAX_WIDTH`].
    pub fn set_size(&self, width: u32, height: u32) {
        let (w, h) = render_size(width, height);
        let mut c = self.control();
        c.width = w;
        c.height = h;
    }

    /// Sans lecture en cours, le spectre retombe à zéro.
    pub fn set_playing(&self, playing: bool) {
        self.control().playing = playing;
    }

    /// Dernière image, si elle est plus récente que `after`.
    pub fn take_frame(&self, after: u64) -> Option<Frame> {
        let mut out = self.output.lock().unwrap_or_else(|e| e.into_inner());
        out.frame.take_if(|f| f.seq > after)
    }

    /// Erreur qui empêche tout rendu (pas de carte graphique…).
    pub fn error(&self) -> Option<String> {
        self.output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .error
            .clone()
    }
}

impl Drop for Visualizer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Taille de rendu : même proportion, largeur limitée, dimensions paires.
fn render_size(width: u32, height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    let scale = (MAX_WIDTH as f32 / width as f32).min(1.);
    let even = |v: f32| ((v as u32).max(2) / 2) * 2;
    (even(width as f32 * scale), even(height as f32 * scale))
}

fn run(
    control: Arc<Mutex<Control>>,
    output: Arc<Mutex<Output>>,
    stop: Arc<AtomicBool>,
    mut spectrum: SpectrumFn,
) {
    let set_error = |e: String| {
        tracing::warn!("visualiseur : {e}");
        output.lock().unwrap_or_else(|e| e.into_inner()).error = Some(e);
    };
    let mut renderer = match Renderer::new() {
        Ok(r) => r,
        Err(e) => return set_error(e.to_string()),
    };
    let mut analysis = Analysis::default();
    let mut raw = [0f32; BANDS];
    let mut last = Instant::now();
    let mut seq = 0u64;
    while !stop.load(Ordering::Relaxed) {
        let start = Instant::now();
        let c = *control.lock().unwrap_or_else(|e| e.into_inner());
        if c.width == 0 {
            std::thread::sleep(Duration::from_millis(50));
            last = Instant::now();
            continue;
        }
        let dt = last.elapsed().as_secs_f32().min(0.1);
        last = start;
        raw.iter_mut().for_each(|b| *b = 0.);
        if c.playing {
            spectrum(&mut raw);
        }
        let inputs = analysis.update(&raw, dt);
        match renderer.render(c.effect, c.width, c.height, &inputs) {
            Ok(pixels) => {
                seq += 1;
                output.lock().unwrap_or_else(|e| e.into_inner()).frame = Some(Frame {
                    width: c.width,
                    height: c.height,
                    pixels,
                    seq,
                });
            }
            Err(e) => {
                set_error(e.to_string());
                return;
            }
        }
        if let Some(rest) = FRAME_TIME.checked_sub(start.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_size_keeps_ratio_and_limits_width() {
        assert_eq!(render_size(1600, 900), (800, 450));
        assert_eq!(render_size(301, 201), (300, 200));
        assert_eq!(render_size(0, 100), (0, 0));
    }
}
