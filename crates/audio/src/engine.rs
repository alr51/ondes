//! Thread du moteur : reçoit les commandes, pilote la sortie, émet les événements.

use std::collections::VecDeque;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ondes_sources::{HttpOptions, SourceSpec, StreamStatus, open_file, open_stream};
use rodio::mixer::Mixer;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};
use tokio::runtime::Handle;

use crate::source::{BufferConfig, Prepared, Shared, prepare};
use crate::{Command, EngineError, Event, PlaybackState, TrackInfo};

/// Intervalle de mise à jour de l'état et de la position.
const TICK: Duration = Duration::from_millis(100);

enum Msg {
    Cmd(Command),
    Loaded {
        generation: u64,
        result: Result<Box<(Prepared, TrackInfo)>, String>,
    },
    Stream {
        generation: u64,
        status: StreamStatus,
    },
    Shutdown,
}

/// Poignée du moteur audio. L'abandonner arrête le thread.
pub struct AudioEngine {
    tx: flume::Sender<Msg>,
    events: flume::Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl AudioEngine {
    /// Démarre le moteur sur la sortie audio par défaut.
    /// `rt` sert aux connexions réseau.
    pub fn start(rt: Handle) -> Result<Self, EngineError> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let engine = Self::spawn(rt, move || {
            // La sortie cpal n'est pas `Send` : elle est créée sur le thread du moteur.
            match DeviceSinkBuilder::open_default_sink() {
                Ok(mut sink) => {
                    sink.log_on_drop(false);
                    let mixer = sink.mixer().clone();
                    let _ = ready_tx.send(Ok(()));
                    Some((mixer, Some(sink)))
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    None
                }
            }
        })?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(engine),
            Ok(Err(e)) => Err(EngineError::NoOutput(e)),
            Err(_) => Err(EngineError::NoOutput("thread audio arrêté".into())),
        }
    }

    /// Démarre le moteur sur un mixeur fourni (tests, sortie fichier…).
    pub fn with_mixer(rt: Handle, mixer: Mixer) -> Result<Self, EngineError> {
        Self::spawn(rt, move || Some((mixer, None)))
    }

    fn spawn(
        rt: Handle,
        output: impl FnOnce() -> Option<(Mixer, Option<MixerDeviceSink>)> + Send + 'static,
    ) -> Result<Self, EngineError> {
        let (tx, rx) = flume::unbounded();
        let (events_tx, events) = flume::unbounded();
        let loader_tx = tx.clone();
        let thread = std::thread::Builder::new()
            .name("ondes-audio".into())
            .spawn(move || {
                let Some((mixer, sink)) = output() else {
                    return;
                };
                Engine {
                    mixer,
                    _sink: sink,
                    rt,
                    tx: loader_tx,
                    events: events_tx,
                    volume: 1.0,
                    state: PlaybackState::Idle,
                    generation: 0,
                    current: None,
                    last_spec: None,
                    last_pos_secs: None,
                    titles: VecDeque::new(),
                    early_title: None,
                }
                .run(rx);
            })?;
        Ok(Self {
            tx,
            events,
            thread: Some(thread),
        })
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(Msg::Cmd(cmd));
    }

    /// Flux d'événements (clonable ; `recv_async` utilisable depuis GPUI).
    pub fn events(&self) -> flume::Receiver<Event> {
        self.events.clone()
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Current {
    player: Player,
    shared: Arc<Shared>,
    info: TrackInfo,
}

struct Engine {
    mixer: Mixer,
    /// Garde la sortie audio ouverte.
    _sink: Option<MixerDeviceSink>,
    rt: Handle,
    tx: flume::Sender<Msg>,
    events: flume::Sender<Event>,
    volume: f32,
    state: PlaybackState,
    /// Incrémenté à chaque changement de source : invalide les messages en retard.
    generation: u64,
    current: Option<Current>,
    /// Dernière source demandée, pour « Lecture » après arrêt ou fin.
    last_spec: Option<SourceSpec>,
    last_pos_secs: Option<u64>,
    /// Titres ICY reçus, à annoncer quand l'audio correspondant sera joué.
    titles: VecDeque<(Instant, String)>,
    /// Titre reçu avant la fin du chargement : annoncé avec la piste.
    early_title: Option<String>,
}

impl Engine {
    fn run(mut self, rx: flume::Receiver<Msg>) {
        loop {
            match rx.recv_timeout(TICK) {
                Ok(Msg::Shutdown) | Err(flume::RecvTimeoutError::Disconnected) => break,
                Ok(Msg::Cmd(cmd)) => self.command(cmd),
                Ok(Msg::Loaded { generation, result }) if generation == self.generation => {
                    self.loaded(result)
                }
                Ok(Msg::Stream { generation, status }) if generation == self.generation => {
                    self.stream_status(status)
                }
                // Message d'une source déjà remplacée : ignoré (et libéré).
                Ok(Msg::Loaded { .. } | Msg::Stream { .. }) => {}
                Err(flume::RecvTimeoutError::Timeout) => {}
            }
            self.tick();
        }
        tracing::debug!("moteur audio arrêté");
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn set_state(&mut self, state: PlaybackState) {
        if self.state != state {
            tracing::debug!(?state, "état");
            self.state = state;
            self.emit(Event::State(state));
        }
    }

    /// Abandonne la source en cours (arrête aussi le réseau et le décodage).
    fn drop_current(&mut self) {
        self.generation += 1;
        self.current = None;
        self.last_pos_secs = None;
        self.titles.clear();
        self.early_title = None;
    }

    fn command(&mut self, cmd: Command) {
        tracing::debug!(?cmd, "commande");
        match cmd {
            Command::Load(spec) => self.load(spec),
            Command::Play => self.play(),
            Command::Pause => self.pause(),
            Command::TogglePlay => {
                if self.state.is_active() {
                    self.pause()
                } else {
                    self.play()
                }
            }
            Command::Stop => {
                self.drop_current();
                self.set_state(PlaybackState::Idle);
            }
            Command::SetVolume(v) => {
                self.volume = v.clamp(0.0, 1.0);
                if let Some(cur) = &self.current {
                    cur.player.set_volume(perceptual(self.volume));
                }
            }
        }
    }

    fn play(&mut self) {
        match &self.current {
            Some(cur) if self.state == PlaybackState::Paused => {
                cur.player.play();
                self.tick();
            }
            Some(_) => {}
            None => {
                if let Some(spec) = self.last_spec.clone() {
                    self.load(spec);
                }
            }
        }
    }

    fn pause(&mut self) {
        let live = self.current.as_ref().is_some_and(|c| c.info.live);
        match (&self.current, self.state) {
            // Un direct mis en pause prendrait du retard : on coupe et on
            // reprendra en direct.
            (Some(_), s) if live && s.is_active() => {
                self.drop_current();
                self.set_state(PlaybackState::Paused);
            }
            (Some(cur), s) if s.is_active() => {
                cur.player.pause();
                self.set_state(PlaybackState::Paused);
            }
            (None, PlaybackState::Loading) => {
                self.drop_current();
                self.set_state(PlaybackState::Paused);
            }
            _ => {}
        }
    }

    fn load(&mut self, spec: SourceSpec) {
        self.drop_current();
        self.last_spec = Some(spec.clone());
        self.set_state(PlaybackState::Loading);

        let generation = self.generation;
        let tx = self.tx.clone();
        let rt = self.rt.clone();
        let spawned = std::thread::Builder::new()
            .name("ondes-load".into())
            .spawn(move || {
                let result = open_and_prepare(&spec, &rt, generation, &tx).map(Box::new);
                if let Err(e) = &result {
                    tracing::warn!(?spec, error = %e, "échec d'ouverture");
                }
                let _ = tx.send(Msg::Loaded { generation, result });
            });
        if let Err(e) = spawned {
            self.set_state(PlaybackState::Idle);
            self.emit(Event::Error(format!("thread de chargement : {e}")));
        }
    }

    fn loaded(&mut self, result: Result<Box<(Prepared, TrackInfo)>, String>) {
        match result {
            Ok(boxed) => {
                let (prepared, info) = *boxed;
                let player = Player::connect_new(&self.mixer);
                player.set_volume(perceptual(self.volume));
                player.append(prepared.source);
                self.emit(Event::Track(info.clone()));
                if let Some(title) = self.early_title.take() {
                    self.emit(Event::NowPlaying(title));
                }
                self.current = Some(Current {
                    player,
                    shared: prepared.shared,
                    info,
                });
                self.set_state(PlaybackState::Buffering);
            }
            Err(message) => {
                self.set_state(PlaybackState::Idle);
                self.emit(Event::Error(message));
            }
        }
    }

    fn stream_status(&mut self, status: StreamStatus) {
        match status {
            StreamStatus::Reconnecting { attempt } => self.emit(Event::Reconnecting { attempt }),
            StreamStatus::Reconnected => tracing::info!("flux reconnecté"),
            StreamStatus::Title(title) => match &self.current {
                Some(cur) => {
                    // Le réseau a de l'avance : on attend que la réserve soit jouée.
                    let per_sec =
                        (cur.info.sample_rate as usize * cur.info.channels as usize).max(1);
                    let ahead = Duration::from_secs_f64(
                        cur.shared.queued_samples() as f64 / per_sec as f64,
                    );
                    self.titles.push_back((Instant::now() + ahead, title));
                }
                None => self.early_title = Some(title),
            },
            StreamStatus::Failed(reason) => {
                self.emit(Event::Error(format!("flux interrompu : {reason}")))
            }
        }
    }

    /// Met à jour l'état (tampon, fin) et la position.
    fn tick(&mut self) {
        let Some(cur) = &self.current else {
            return;
        };
        if cur.player.empty() {
            self.current = None;
            self.last_pos_secs = None;
            self.set_state(PlaybackState::Ended);
            return;
        }
        if self.state == PlaybackState::Paused {
            return;
        }
        let state = if cur.shared.is_starved() {
            PlaybackState::Buffering
        } else {
            PlaybackState::Playing
        };
        while let Some((due, _)) = self.titles.front()
            && *due <= Instant::now()
        {
            if let Some((_, title)) = self.titles.pop_front() {
                self.emit(Event::NowPlaying(title));
            }
        }
        let Some(cur) = &self.current else {
            return;
        };
        let pos = cur.player.get_pos();
        self.set_state(state);
        if self.last_pos_secs != Some(pos.as_secs()) {
            self.last_pos_secs = Some(pos.as_secs());
            self.emit(Event::Position(pos));
        }
    }
}

fn open_and_prepare(
    spec: &SourceSpec,
    rt: &Handle,
    generation: u64,
    tx: &flume::Sender<Msg>,
) -> Result<(Prepared, TrackInfo), String> {
    let opened = match spec {
        SourceSpec::File(path) => open_file(path),
        SourceSpec::Url(url) => {
            let tx = tx.clone();
            open_stream(rt, url, HttpOptions::default(), move |status| {
                let _ = tx.send(Msg::Stream { generation, status });
            })
        }
    }
    .map_err(|e| e.to_string())?;

    let title = opened.title.clone();
    let live = opened.live;
    let format = opened.hint.clone().or_else(|| opened.mime.clone());
    let prepared = prepare(opened, BufferConfig::for_source(live))?;
    let info = TrackInfo {
        spec: spec.clone(),
        title,
        live,
        duration: prepared.duration,
        format,
        sample_rate: prepared.sample_rate,
        channels: prepared.channels,
    };
    Ok((prepared, info))
}

/// Courbe de volume approximativement perceptive.
fn perceptual(v: f32) -> f32 {
    v * v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZero;
    use std::time::Instant;

    fn wav_file(n: usize, rate: u32) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("ondes-engine-{}-{}.wav", std::process::id(), n));
        std::fs::write(&path, crate::source::wav(n, rate)).unwrap();
        path
    }

    /// Moteur relié à un mixeur virtuel tiré en temps réel par un thread.
    fn engine() -> (
        AudioEngine,
        tokio::runtime::Runtime,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let (mixer, mut out) =
            rodio::mixer::mixer(NonZero::new(2).unwrap(), NonZero::new(8_000).unwrap());
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop2 = stop.clone();
        std::thread::spawn(move || {
            // ~ temps réel : 16 000 échantillons/s en blocs de 10 ms.
            while !stop2.load(std::sync::atomic::Ordering::Relaxed) {
                for _ in 0..160 {
                    out.next();
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let engine = AudioEngine::with_mixer(rt.handle().clone(), mixer).unwrap();
        (engine, rt, stop)
    }

    fn wait_for(events: &flume::Receiver<Event>, pred: impl Fn(&Event) -> bool) -> Vec<Event> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            if let Ok(ev) = events.recv_timeout(Duration::from_millis(50)) {
                let done = pred(&ev);
                seen.push(ev);
                if done {
                    return seen;
                }
            }
        }
        panic!("événement attendu non reçu ; vus : {seen:?}");
    }

    #[test]
    fn plays_local_file_to_the_end() {
        let path = wav_file(4_000, 8_000); // 0,5 s
        let (engine, _rt, stop) = engine();
        let events = engine.events();
        engine.send(Command::Load(SourceSpec::File(path.clone())));

        let seen = wait_for(&events, |e| *e == Event::State(PlaybackState::Ended));
        assert_eq!(seen[0], Event::State(PlaybackState::Loading));
        let track = seen
            .iter()
            .find_map(|e| match e {
                Event::Track(t) => Some(t),
                _ => None,
            })
            .expect("piste annoncée");
        assert_eq!(track.sample_rate, 8_000);
        assert!(!track.live);
        assert!(seen.contains(&Event::State(PlaybackState::Playing)));

        // « Lecture » après la fin relance la même source.
        engine.send(Command::Play);
        wait_for(&events, |e| *e == Event::State(PlaybackState::Playing));
        engine.send(Command::Pause);
        wait_for(&events, |e| *e == Event::State(PlaybackState::Paused));
        engine.send(Command::Stop);
        wait_for(&events, |e| *e == Event::State(PlaybackState::Idle));

        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn reports_open_errors() {
        let (engine, _rt, stop) = engine();
        let events = engine.events();
        engine.send(Command::Load(SourceSpec::File("/nonexistent/x.mp3".into())));
        let seen = wait_for(&events, |e| matches!(e, Event::Error(_)));
        assert!(seen.contains(&Event::State(PlaybackState::Idle)));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
