//! Décodage sur un thread dédié et source non bloquante pour la sortie audio.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use ondes_sources::Opened;
use rodio::{ChannelCount, Decoder, Sample, SampleRate, Source};

/// Taille d'un bloc d'échantillons transmis par le décodeur.
const CHUNK_FRAMES: usize = 2048;
/// Durée de silence produite d'un coup pendant une mise en tampon.
const SILENCE_FRAMES: usize = 512;

/// Réglages de tampon.
#[derive(Debug, Clone, Copy)]
pub struct BufferConfig {
    /// Audio à accumuler avant de (re)commencer à jouer.
    pub prebuffer: Duration,
    /// Audio décodé maximal gardé en avance.
    pub capacity: Duration,
}

impl BufferConfig {
    pub fn for_source(live: bool) -> Self {
        if live {
            Self {
                prebuffer: Duration::from_millis(2_000),
                capacity: Duration::from_secs(10),
            }
        } else {
            Self {
                prebuffer: Duration::from_millis(250),
                capacity: Duration::from_secs(3),
            }
        }
    }
}

/// État partagé entre la source (thread audio) et le moteur.
#[derive(Debug, Default)]
pub struct Shared {
    /// Échantillons en attente dans la file.
    queued: AtomicUsize,
    /// La source joue du silence faute de données.
    starved: AtomicBool,
}

impl Shared {
    pub fn is_starved(&self) -> bool {
        self.starved.load(Ordering::Relaxed)
    }

    /// Échantillons décodés en attente de lecture.
    pub fn queued_samples(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }
}

/// Résultat de la préparation : de quoi jouer, et quoi afficher.
pub struct Prepared {
    pub source: StreamingSource,
    pub shared: Arc<Shared>,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration: Option<Duration>,
}

/// Construit le décodeur (bloquant : lit l'en-tête du flux), puis lance le
/// thread de décodage. À appeler hors du thread audio.
/// `start` : position de départ (0 = début), pour se déplacer dans un fichier.
pub fn prepare(opened: Opened, config: BufferConfig, start: Duration) -> Result<Prepared, String> {
    let Opened {
        reader,
        hint,
        mime,
        byte_len,
        live,
        ..
    } = opened;

    let mut builder = Decoder::builder()
        .with_data(reader)
        .with_seekable(!live)
        .with_gapless(true);
    if let Some(hint) = &hint {
        builder = builder.with_hint(hint);
    }
    if let Some(mime) = &mime {
        builder = builder.with_mime_type(mime);
    }
    if let Some(len) = byte_len {
        builder = builder.with_byte_len(len);
    }
    let decoder = builder
        .build()
        .map_err(|e| unsupported_message(&e, mime.as_deref(), hint.as_deref()))?;

    let mut decoder = decoder;
    if !start.is_zero() && !live {
        // Échec possible sur certains formats : on repart alors du début.
        if let Err(e) = decoder.try_seek(start) {
            tracing::warn!(error = %e, "déplacement impossible");
        }
    }
    let channels = decoder.channels();
    let sample_rate = decoder.sample_rate();
    let duration = if live { None } else { decoder.total_duration() };

    let samples_per_sec = sample_rate.get() as usize * channels.get() as usize;
    let chunk_len = CHUNK_FRAMES * channels.get() as usize;
    let capacity_chunks =
        (samples_per_sec * config.capacity.as_millis() as usize / 1000 / chunk_len).max(2);
    let prebuffer = samples_per_sec * config.prebuffer.as_millis() as usize / 1000;

    let shared = Arc::new(Shared::default());
    let (tx, rx) = flume::bounded::<Vec<Sample>>(capacity_chunks);

    let shared_dec = shared.clone();
    std::thread::Builder::new()
        .name("ondes-decode".into())
        .spawn(move || decode_loop(decoder, chunk_len, tx, &shared_dec))
        .map_err(|e| format!("thread de décodage : {e}"))?;

    Ok(Prepared {
        source: StreamingSource::new(
            rx,
            shared.clone(),
            channels,
            sample_rate,
            duration,
            prebuffer,
        ),
        shared,
        sample_rate: sample_rate.get(),
        channels: channels.get(),
        duration,
    })
}

/// Message clair quand le décodeur refuse la source.
fn unsupported_message(
    error: &rodio::decoder::DecoderError,
    mime: Option<&str>,
    hint: Option<&str>,
) -> String {
    let format = mime.or(hint).unwrap_or_default().to_lowercase();
    if format.contains("opus") || format == "ogg" || format.ends_with("/ogg") {
        // symphonia lit Ogg Vorbis mais pas Ogg Opus, fréquent chez les radios.
        return "format non pris en charge : probablement de l'Opus (seul Ogg Vorbis est lu)"
            .into();
    }
    match error {
        rodio::decoder::DecoderError::UnrecognizedFormat if format.is_empty() => {
            "format audio non reconnu".into()
        }
        rodio::decoder::DecoderError::UnrecognizedFormat => {
            format!("format non pris en charge ({format})")
        }
        other => format!("décodage impossible : {other}"),
    }
}

/// Décode jusqu'à la fin ou jusqu'à ce que la source soit abandonnée.
fn decode_loop<I>(decoder: I, chunk_len: usize, tx: flume::Sender<Vec<Sample>>, shared: &Shared)
where
    I: Iterator<Item = Sample>,
{
    let mut chunk = Vec::with_capacity(chunk_len);
    for sample in decoder {
        chunk.push(sample);
        if chunk.len() == chunk_len {
            let full = std::mem::replace(&mut chunk, Vec::with_capacity(chunk_len));
            shared.queued.fetch_add(full.len(), Ordering::Relaxed);
            if tx.send(full).is_err() {
                return; // source abandonnée
            }
        }
    }
    if !chunk.is_empty() {
        shared.queued.fetch_add(chunk.len(), Ordering::Relaxed);
        let _ = tx.send(chunk);
    }
    tracing::debug!("fin du décodage");
}

/// Source rodio alimentée par le thread de décodage. Ne bloque jamais :
/// produit du silence tant que le tampon n'est pas suffisant.
pub struct StreamingSource {
    rx: flume::Receiver<Vec<Sample>>,
    shared: Arc<Shared>,
    current: Vec<Sample>,
    pos: usize,
    silence_left: usize,
    buffering: bool,
    prebuffer: usize,
    channels: ChannelCount,
    sample_rate: SampleRate,
    duration: Option<Duration>,
}

impl StreamingSource {
    fn new(
        rx: flume::Receiver<Vec<Sample>>,
        shared: Arc<Shared>,
        channels: ChannelCount,
        sample_rate: SampleRate,
        duration: Option<Duration>,
        prebuffer: usize,
    ) -> Self {
        shared.starved.store(true, Ordering::Relaxed);
        Self {
            rx,
            shared,
            current: Vec::new(),
            pos: 0,
            silence_left: 0,
            buffering: true,
            prebuffer,
            channels,
            sample_rate,
            duration,
        }
    }

    fn start_silence(&mut self) {
        self.buffering = true;
        self.shared.starved.store(true, Ordering::Relaxed);
        self.silence_left = SILENCE_FRAMES * self.channels.get() as usize;
    }
}

impl Iterator for StreamingSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        loop {
            if self.silence_left > 0 {
                self.silence_left -= 1;
                return Some(0.0);
            }
            if let Some(&s) = self.current.get(self.pos) {
                self.pos += 1;
                return Some(s);
            }

            // Bloc courant épuisé : en mise en tampon, attendre d'avoir assez d'avance
            // (sauf si le décodage est terminé : on joue ce qui reste).
            let finished = self.rx.is_disconnected();
            if self.buffering
                && !finished
                && self.shared.queued.load(Ordering::Relaxed) < self.prebuffer
            {
                self.start_silence();
                continue;
            }

            match self.rx.try_recv() {
                Ok(chunk) => {
                    self.shared.queued.fetch_sub(chunk.len(), Ordering::Relaxed);
                    if self.buffering {
                        self.buffering = false;
                        self.shared.starved.store(false, Ordering::Relaxed);
                    }
                    self.current = chunk;
                    self.pos = 0;
                }
                Err(flume::TryRecvError::Empty) => self.start_silence(),
                Err(flume::TryRecvError::Disconnected) => {
                    self.shared.starved.store(false, Ordering::Relaxed);
                    return None;
                }
            }
        }
    }
}

impl Source for StreamingSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }
}

#[cfg(test)]
/// WAV PCM 16 bits mono (tests) de `n` échantillons.
pub(crate) fn wav(n: usize, rate: u32) -> Vec<u8> {
    let data_len = (n * 2) as u32;
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..n {
        let v = if i % 2 == 0 { 8_000i16 } else { -8_000 };
        w.extend_from_slice(&v.to_le_bytes());
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::num::NonZero;

    fn nz16(v: u16) -> ChannelCount {
        NonZero::new(v).unwrap()
    }

    fn nz32(v: u32) -> SampleRate {
        NonZero::new(v).unwrap()
    }

    fn source_with(prebuffer: usize) -> (StreamingSource, flume::Sender<Vec<Sample>>, Arc<Shared>) {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = flume::bounded(16);
        let src = StreamingSource::new(rx, shared.clone(), nz16(2), nz32(48_000), None, prebuffer);
        (src, tx, shared)
    }

    fn push(tx: &flume::Sender<Vec<Sample>>, shared: &Shared, chunk: Vec<Sample>) {
        shared.queued.fetch_add(chunk.len(), Ordering::Relaxed);
        tx.send(chunk).unwrap();
    }

    #[test]
    fn plays_silence_until_prebuffered() {
        let (mut src, tx, shared) = source_with(4);
        push(&tx, &shared, vec![1.0, 1.0]);
        // 2 échantillons < 4 : silence, par trames complètes.
        let first: Vec<_> = (&mut src).take(SILENCE_FRAMES * 2).collect();
        assert!(first.iter().all(|&s| s == 0.0));
        assert!(shared.is_starved());

        push(&tx, &shared, vec![2.0, 2.0]);
        assert_eq!(src.next(), Some(1.0));
        assert!(!shared.is_starved());
        assert_eq!((&mut src).take(3).collect::<Vec<_>>(), vec![1.0, 2.0, 2.0]);

        // Plus rien : retour en mise en tampon.
        assert_eq!(src.next(), Some(0.0));
        assert!(shared.is_starved());
    }

    #[test]
    fn drains_remaining_samples_when_decoder_finished() {
        let (src, tx, shared) = source_with(1_000);
        push(&tx, &shared, vec![0.5, 0.5]);
        drop(tx); // décodage terminé : pas d'attente du prébuffer
        assert_eq!(src.collect::<Vec<_>>(), vec![0.5, 0.5]);
    }

    #[test]
    fn decodes_wav_end_to_end() {
        let bytes = wav(10_000, 8_000);
        let opened = Opened {
            byte_len: Some(bytes.len() as u64),
            reader: Box::new(Cursor::new(bytes)),
            title: "test".into(),
            hint: Some("wav".into()),
            mime: None,
            live: false,
        };
        let prepared = prepare(opened, BufferConfig::for_source(false), Duration::ZERO).unwrap();
        assert_eq!(prepared.channels, 1);
        assert_eq!(prepared.sample_rate, 8_000);
        let d = prepared.duration.unwrap().as_millis();
        assert!((1_240..=1_260).contains(&d), "durée {d} ms");

        // Sans sortie audio : on tire les échantillons à la main.
        let samples: Vec<_> = prepared.source.filter(|&s| s != 0.0).collect();
        assert_eq!(samples.len(), 10_000);
        assert!(
            samples
                .iter()
                .all(|&s| (s.abs() - 8_000.0 / 32_768.0).abs() < 1e-3)
        );
    }

    #[test]
    fn rejects_garbage() {
        let opened = Opened {
            reader: Box::new(Cursor::new(vec![0u8; 4096])),
            title: "x".into(),
            hint: None,
            mime: None,
            byte_len: Some(4096),
            live: false,
        };
        assert!(prepare(opened, BufferConfig::for_source(false), Duration::ZERO).is_err());
    }
}
