//! Thread du moteur : reçoit les commandes, pilote la sortie, émet les événements.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ondes_sources::{HttpOptions, Recorder, SourceSpec, StreamStatus, open_file, open_stream};
use rodio::mixer::Mixer;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};
use tokio::runtime::Handle;

use crate::dsp::{Analyzer, DspShared};
use crate::source::{BufferConfig, Prepared, Shared, prepare};
use crate::{
    Command, EngineError, Event, NextTrack, PlaybackState, ReplayGain, ReplayGainMode, TrackInfo,
};

/// Intervalle de mise à jour de l'état et de la position.
const TICK: Duration = Duration::from_millis(100);
/// Le morceau suivant est préparé quand il reste ce temps (plus le fondu).
const PRELOAD_AHEAD: Duration = Duration::from_secs(10);
/// Sans fondu, le suivant est mis à la suite dans le lecteur à ce moment-là.
const GAPLESS_QUEUE_AT: Duration = Duration::from_secs(2);
/// Fondu le plus court possible (morceau presque fini quand il est prêt).
const MIN_FADE: Duration = Duration::from_millis(300);

/// Source ouverte et prête à jouer.
struct Loaded {
    prepared: Prepared,
    info: TrackInfo,
    recorder: Option<Recorder>,
}

enum Msg {
    Cmd(Command),
    Loaded {
        generation: u64,
        result: Result<Box<Loaded>, String>,
    },
    NextLoaded {
        generation: u64,
        result: Result<Box<Loaded>, String>,
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
    dsp: Arc<DspShared>,
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
        let dsp = Arc::new(DspShared::default());
        let engine_dsp = dsp.clone();
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
                    dsp: engine_dsp,
                    volume: 1.0,
                    replay_gain: ReplayGainMode::Off,
                    crossfade: Duration::ZERO,
                    state: PlaybackState::Idle,
                    generation: 0,
                    next_generation: 0,
                    current: None,
                    fading: Vec::new(),
                    next: None,
                    last_spec: None,
                    last_pos_secs: None,
                    titles: VecDeque::new(),
                    seek: None,
                    early_title: None,
                }
                .run(rx);
            })?;
        Ok(Self {
            tx,
            events,
            dsp,
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

    /// Spectre de ce qui est joué, pour un visualiseur.
    pub fn analyzer(&self) -> Analyzer {
        Analyzer(self.dsp.clone())
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

/// Fondu en cours (entrée ou sortie).
#[derive(Debug, Clone, Copy)]
struct Fade {
    start: Instant,
    duration: Duration,
}

impl Fade {
    fn new(duration: Duration) -> Self {
        Self {
            start: Instant::now(),
            duration: duration.max(MIN_FADE),
        }
    }

    /// Avancement, de 0 à 1.
    fn progress(&self) -> f32 {
        (self.start.elapsed().as_secs_f32() / self.duration.as_secs_f32()).min(1.)
    }
}

struct Current {
    player: Player,
    shared: Arc<Shared>,
    info: TrackInfo,
    /// Position de départ (après un déplacement).
    offset: Duration,
    recorder: Option<Recorder>,
    /// Montée du volume au début d'un fondu enchaîné.
    fade_in: Option<Fade>,
}

impl Current {
    fn position(&self) -> Duration {
        let rate = f64::from(self.info.sample_rate.max(1));
        self.offset + Duration::from_secs_f64(self.shared.played_frames() as f64 / rate)
    }
}

/// Morceau précédent qui s'éteint pendant un fondu enchaîné.
struct Fading {
    player: Player,
    shared: Arc<Shared>,
    replay_gain: Option<ReplayGain>,
    fade: Fade,
}

enum NextState {
    /// À précharger le moment venu.
    Wanted,
    Loading,
    Ready(Box<Loaded>),
    /// Mis à la suite dans le lecteur en cours : démarre à la fin de celui-ci.
    Queued {
        shared: Arc<Shared>,
        info: TrackInfo,
    },
}

struct Next {
    track: NextTrack,
    state: NextState,
}

/// Déplacement en cours : la source est rouverte à `pos`.
struct SeekTarget {
    pos: Duration,
    paused: bool,
    info: TrackInfo,
}

struct Engine {
    mixer: Mixer,
    /// Garde la sortie audio ouverte.
    _sink: Option<MixerDeviceSink>,
    rt: Handle,
    tx: flume::Sender<Msg>,
    events: flume::Sender<Event>,
    dsp: Arc<DspShared>,
    volume: f32,
    replay_gain: ReplayGainMode,
    crossfade: Duration,
    state: PlaybackState,
    /// Incrémenté à chaque changement de source : invalide les messages en retard.
    generation: u64,
    /// Idem pour le préchargement du morceau suivant.
    next_generation: u64,
    current: Option<Current>,
    fading: Vec<Fading>,
    next: Option<Next>,
    /// Dernière source demandée, pour « Lecture » après arrêt ou fin.
    last_spec: Option<SourceSpec>,
    last_pos_secs: Option<u64>,
    /// Titres ICY reçus, à annoncer quand l'audio correspondant sera joué.
    titles: VecDeque<(Instant, String)>,
    seek: Option<SeekTarget>,
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
                Ok(Msg::NextLoaded { generation, result })
                    if generation == self.next_generation =>
                {
                    self.next_loaded(result)
                }
                Ok(Msg::Stream { generation, status }) if generation == self.generation => {
                    self.stream_status(status)
                }
                // Message d'une source déjà remplacée : ignoré (et libéré).
                Ok(Msg::Loaded { .. } | Msg::NextLoaded { .. } | Msg::Stream { .. }) => {}
                Err(flume::RecvTimeoutError::Timeout) => {}
            }
            self.tick();
        }
        self.stop_recording();
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
    /// Le morceau suivant demandé est gardé, à précharger de nouveau.
    fn drop_current(&mut self) {
        self.stop_recording();
        self.generation += 1;
        self.next_generation += 1;
        self.current = None;
        self.fading.clear();
        if let Some(next) = &mut self.next {
            next.state = NextState::Wanted;
        }
        self.last_pos_secs = None;
        self.titles.clear();
        self.early_title = None;
        self.seek = None;
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
                self.next = None;
                self.set_state(PlaybackState::Idle);
            }
            Command::Seek(pos) => self.seek_to(pos),
            Command::SetVolume(v) => {
                self.volume = v.clamp(0.0, 1.0);
                self.apply_gains();
            }
            Command::SetReplayGain(mode) => {
                self.replay_gain = mode;
                self.apply_gains();
            }
            Command::SetEqualizer(eq) => self.dsp.set_equalizer(eq),
            Command::SetCrossfade(d) => self.crossfade = d.min(Duration::from_secs(15)),
            Command::SetNext(track) => self.set_next(track),
            Command::Record(Some(base)) => self.start_recording(base),
            Command::Record(None) => self.stop_recording(),
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
                // Le morceau qui s'éteignait est coupé net.
                self.fading.clear();
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
        self.next = None;
        self.last_spec = Some(spec.clone());
        self.set_state(PlaybackState::Loading);
        self.spawn_loader(spec, Duration::ZERO, false);
    }

    /// Rouvre la source en cours à `pos`, en gardant l'état pause.
    fn seek_to(&mut self, pos: Duration) {
        let Some(cur) = &self.current else {
            return;
        };
        if cur.info.live {
            return;
        }
        let pos = cur.info.duration.map_or(pos, |d| pos.min(d));
        let target = SeekTarget {
            pos,
            paused: self.state == PlaybackState::Paused,
            info: cur.info.clone(),
        };
        let spec = target.info.spec.clone();
        self.drop_current();
        self.seek = Some(target);
        self.emit(Event::Position(pos));
        self.spawn_loader(spec, pos, false);
    }

    /// Ouvre une source sur un thread à part. `next` : préchargement du
    /// morceau suivant (réponse dans [`Msg::NextLoaded`]).
    fn spawn_loader(&mut self, spec: SourceSpec, start: Duration, next: bool) {
        let generation = if next {
            self.next_generation
        } else {
            self.generation
        };
        let tx = self.tx.clone();
        let rt = self.rt.clone();
        let dsp = self.dsp.clone();
        let spawned = std::thread::Builder::new()
            .name("ondes-load".into())
            .spawn(move || {
                // Les statuts réseau d'un préchargement (fichier) n'ont pas lieu d'être.
                let status_generation = if next { u64::MAX } else { generation };
                let result =
                    open_and_prepare(&spec, &rt, status_generation, &tx, start, dsp).map(Box::new);
                if let Err(e) = &result {
                    tracing::warn!(?spec, error = %e, "échec d'ouverture");
                }
                let _ = tx.send(if next {
                    Msg::NextLoaded { generation, result }
                } else {
                    Msg::Loaded { generation, result }
                });
            });
        if let Err(e) = spawned {
            if next {
                self.next = None;
            } else {
                self.set_state(PlaybackState::Idle);
            }
            self.emit(Event::Error(format!("thread de chargement : {e}")));
        }
    }

    fn loaded(&mut self, result: Result<Box<Loaded>, String>) {
        match result {
            Ok(boxed) => {
                let Loaded {
                    prepared,
                    info,
                    recorder,
                } = *boxed;
                let player = Player::connect_new(&self.mixer);
                prepared.shared.set_gain(self.base_gain(&info));
                player.append(prepared.source);
                // Après un déplacement : même piste, on ne la réannonce pas.
                let (offset, paused) = match self.seek.take() {
                    Some(seek) => (seek.pos, seek.paused),
                    None => {
                        self.emit(Event::Track(info.clone()));
                        if let Some(title) = self.early_title.take() {
                            self.emit(Event::NowPlaying(title));
                        }
                        (Duration::ZERO, false)
                    }
                };
                if paused {
                    player.pause();
                }
                self.current = Some(Current {
                    player,
                    shared: prepared.shared,
                    info,
                    offset,
                    recorder,
                    fade_in: None,
                });
                self.last_pos_secs = None;
                self.set_state(if paused {
                    PlaybackState::Paused
                } else {
                    PlaybackState::Buffering
                });
            }
            Err(message) => {
                self.set_state(PlaybackState::Idle);
                self.emit(Event::Error(message));
            }
        }
    }

    fn set_next(&mut self, track: Option<NextTrack>) {
        if let Some(next) = &self.next {
            if Some(&next.track) == track.as_ref() {
                return;
            }
            // Déjà dans le lecteur : rodio ne permet pas de l'en retirer.
            if matches!(next.state, NextState::Queued { .. }) {
                tracing::debug!("morceau suivant déjà enchaîné, changement ignoré");
                return;
            }
        }
        self.next_generation += 1;
        self.next = track.map(|track| Next {
            track,
            state: NextState::Wanted,
        });
    }

    fn next_loaded(&mut self, result: Result<Box<Loaded>, String>) {
        let Some(next) = &mut self.next else {
            return;
        };
        match result {
            Ok(loaded) => next.state = NextState::Ready(loaded),
            // L'erreur sera signalée quand l'UI le chargera elle-même.
            Err(_) => self.next = None,
        }
    }

    /// Gain d'une source : volume et ReplayGain.
    fn base_gain(&self, info: &TrackInfo) -> f32 {
        perceptual(self.volume) * replay_gain_factor(info.replay_gain.as_ref(), self.replay_gain)
    }

    /// Met à jour le gain visé de chaque source (lissé par la source).
    fn apply_gains(&self) {
        if let Some(cur) = &self.current {
            let fade = cur.fade_in.map_or(1., |f| f.progress());
            cur.shared.set_gain(self.base_gain(&cur.info) * fade);
        }
        if let Some(Next {
            state: NextState::Queued { shared, info },
            ..
        }) = &self.next
        {
            shared.set_gain(self.base_gain(info));
        }
        for f in &self.fading {
            let base = perceptual(self.volume)
                * replay_gain_factor(f.replay_gain.as_ref(), self.replay_gain);
            f.shared.set_gain(base * (1. - f.fade.progress()));
        }
    }

    /// Prépare, met à la suite ou lance en fondu le morceau suivant.
    fn schedule_next(&mut self) {
        enum Action {
            Preload(SourceSpec),
            Fade(Option<Duration>),
            Queue,
        }
        let action = {
            let (Some(cur), Some(next)) = (&self.current, &self.next) else {
                return;
            };
            if cur.info.live {
                return;
            }
            let remaining = cur.info.duration.map(|d| d.saturating_sub(cur.position()));
            let fade = !self.crossfade.is_zero() && next.track.crossfade && remaining.is_some();
            match &next.state {
                NextState::Wanted
                    if remaining.is_none_or(|r| r <= self.crossfade + PRELOAD_AHEAD) =>
                {
                    Action::Preload(next.track.spec.clone())
                }
                NextState::Ready(_) if fade && remaining.is_some_and(|r| r <= self.crossfade) => {
                    Action::Fade(remaining)
                }
                NextState::Ready(_) if !fade && remaining.is_none_or(|r| r <= GAPLESS_QUEUE_AT) => {
                    Action::Queue
                }
                _ => return,
            }
        };
        match action {
            Action::Preload(spec) => {
                if let Some(next) = &mut self.next {
                    next.state = NextState::Loading;
                }
                self.spawn_loader(spec, Duration::ZERO, true);
            }
            Action::Fade(remaining) => self.start_next(remaining),
            Action::Queue => self.queue_next(),
        }
    }

    /// Met le morceau préchargé à la suite dans le lecteur en cours : il
    /// démarrera sans blanc à la fin de celui-ci.
    fn queue_next(&mut self) {
        let Some(cur) = &self.current else {
            return;
        };
        let Some(Next {
            track,
            state: NextState::Ready(loaded),
        }) = self.next.take()
        else {
            return;
        };
        let Loaded { prepared, info, .. } = *loaded;
        prepared.shared.set_gain(self.base_gain(&info));
        cur.player.append(prepared.source);
        self.next = Some(Next {
            track,
            state: NextState::Queued {
                shared: prepared.shared,
                info,
            },
        });
    }

    /// Lance le morceau préchargé sur un nouveau lecteur. `fade` : durée
    /// restante du morceau en cours, à fondre avec le début du suivant.
    fn start_next(&mut self, fade: Option<Duration>) {
        let Some(Next {
            state: NextState::Ready(loaded),
            ..
        }) = self.next.take()
        else {
            return;
        };
        let Loaded {
            prepared,
            info,
            recorder,
        } = *loaded;
        let fade = fade.map(|d| Fade::new(d.min(self.crossfade)));
        if let Some(old) = self.current.take() {
            match fade {
                Some(fade) => self.fading.push(Fading {
                    player: old.player,
                    shared: old.shared,
                    replay_gain: old.info.replay_gain,
                    fade,
                }),
                None => drop(old),
            }
        }
        let player = Player::connect_new(&self.mixer);
        prepared.shared.set_gain(if fade.is_some() {
            0.
        } else {
            self.base_gain(&info)
        });
        player.append(prepared.source);
        self.advanced(Current {
            player,
            shared: prepared.shared,
            info,
            offset: Duration::ZERO,
            recorder,
            fade_in: fade,
        });
    }

    /// Le morceau suivant est devenu le morceau en cours.
    fn advanced(&mut self, current: Current) {
        tracing::debug!(title = %current.info.title, "morceau suivant");
        self.last_spec = Some(current.info.spec.clone());
        self.last_pos_secs = None;
        self.titles.clear();
        self.emit(Event::Advanced(current.info.clone()));
        self.current = Some(current);
        self.apply_gains();
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

    fn start_recording(&mut self, base: PathBuf) {
        self.stop_recording();
        let Some(cur) = &self.current else {
            return;
        };
        let Some(recorder) = &cur.recorder else {
            self.emit(Event::Error("seul un direct peut être enregistré".into()));
            return;
        };
        let mut path = OsString::from(base);
        path.push(".");
        path.push(extension(cur.info.format.as_deref()));
        let path = PathBuf::from(path);
        match recorder.start(&path) {
            Ok(()) => {
                tracing::info!(path = %path.display(), "enregistrement");
                self.emit(Event::RecordingStarted(path));
            }
            Err(e) => self.emit(Event::Error(format!(
                "enregistrement impossible ({}) : {e}",
                path.display()
            ))),
        }
    }

    fn stop_recording(&mut self) {
        let done = self
            .current
            .as_ref()
            .and_then(|c| c.recorder.as_ref())
            .and_then(Recorder::stop);
        if let Some(done) = done {
            tracing::info!(path = %done.path.display(), bytes = done.bytes, "fin d'enregistrement");
            self.emit(Event::RecordingStopped(done));
        }
    }

    /// Met à jour l'état (tampon, fin, enchaînements) et la position.
    fn tick(&mut self) {
        self.fading
            .retain(|f| !f.player.empty() && f.fade.progress() < 1.);
        if self
            .current
            .as_ref()
            .and_then(|c| c.recorder.as_ref())
            .is_some_and(Recorder::failed)
        {
            self.stop_recording();
        }

        // Morceau suivant mis à la suite : il vient de commencer.
        if let Some(Next {
            state: NextState::Queued { shared, .. },
            ..
        }) = &self.next
            && shared.has_started()
            && let Some(old) = self.current.take()
            && let Some(Next {
                state: NextState::Queued { shared, info },
                ..
            }) = self.next.take()
        {
            self.advanced(Current {
                player: old.player,
                shared,
                info,
                offset: Duration::ZERO,
                recorder: None,
                fade_in: None,
            });
        }

        let Some(cur) = &self.current else {
            return;
        };
        if cur.player.empty() {
            // Préchargé mais pas encore lancé (durée inconnue…) : on enchaîne.
            if matches!(
                self.next,
                Some(Next {
                    state: NextState::Ready(_),
                    ..
                })
            ) {
                self.start_next(None);
                return;
            }
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
        if let Some(cur) = &mut self.current
            && cur.fade_in.is_some_and(|f| f.progress() >= 1.)
        {
            cur.fade_in = None;
        }
        while let Some((due, _)) = self.titles.front()
            && *due <= Instant::now()
        {
            if let Some((_, title)) = self.titles.pop_front() {
                self.emit(Event::NowPlaying(title));
            }
        }
        self.schedule_next();
        self.apply_gains();
        let Some(cur) = &self.current else {
            return;
        };
        let pos = cur.position();
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
    start: Duration,
    dsp: Arc<DspShared>,
) -> Result<Loaded, String> {
    let mut opened = match spec {
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
    let replay_gain = opened.replay_gain;
    let recorder = opened.recorder.take();
    let prepared = prepare(opened, BufferConfig::for_source(live), start, dsp)?;
    let info = TrackInfo {
        spec: spec.clone(),
        title,
        live,
        duration: prepared.duration,
        format,
        sample_rate: prepared.sample_rate,
        channels: prepared.channels,
        replay_gain,
    };
    Ok(Loaded {
        prepared,
        info,
        recorder,
    })
}

/// Courbe de volume approximativement perceptive.
fn perceptual(v: f32) -> f32 {
    v * v
}

/// Facteur d'amplitude ReplayGain, limité pour ne pas écrêter.
fn replay_gain_factor(rg: Option<&ReplayGain>, mode: ReplayGainMode) -> f32 {
    let Some(rg) = rg else {
        return 1.;
    };
    let (gain, peak) = match mode {
        ReplayGainMode::Off => return 1.,
        ReplayGainMode::Track => (
            rg.track_gain.or(rg.album_gain),
            rg.track_peak.or(rg.album_peak),
        ),
        ReplayGainMode::Album => (
            rg.album_gain.or(rg.track_gain),
            rg.album_peak.or(rg.track_peak),
        ),
    };
    let Some(gain) = gain else {
        return 1.;
    };
    let factor = 10f32.powf(gain.clamp(-24., 18.) / 20.);
    match peak.filter(|&p| p > 0.) {
        Some(peak) => factor.min(1. / peak),
        // Pas de crête connue : pas d'amplification.
        None => factor.min(1.),
    }
}

/// Extension d'un enregistrement d'après le format du flux.
fn extension(format: Option<&str>) -> &'static str {
    match format.unwrap_or_default() {
        "mp3" | "audio/mpeg" | "audio/mp3" => "mp3",
        "aac" | "audio/aac" | "audio/aacp" | "audio/x-aac" => "aac",
        "m4a" | "audio/mp4" => "m4a",
        "ogg" | "audio/ogg" | "application/ogg" => "ogg",
        "flac" | "audio/flac" => "flac",
        _ => "audio",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZero;
    use std::time::Instant;

    fn wav_file(n: usize, rate: u32) -> std::path::PathBuf {
        // Nom unique : les tests tournent en parallèle et suppriment leurs fichiers.
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ondes-engine-{}-{id}-{n}.wav", std::process::id()));
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
    fn seeks_and_keeps_pause_state() {
        let path = wav_file(16_000, 8_000); // 2 s
        let (engine, _rt, stop) = engine();
        let events = engine.events();
        engine.send(Command::Load(SourceSpec::File(path.clone())));
        wait_for(&events, |e| *e == Event::State(PlaybackState::Playing));

        engine.send(Command::Pause);
        wait_for(&events, |e| *e == Event::State(PlaybackState::Paused));
        engine.send(Command::Seek(Duration::from_millis(1_500)));
        let seen = wait_for(&events, |e| {
            *e == Event::Position(Duration::from_millis(1_500))
        });
        // Pas de nouvelle piste annoncée pour un simple déplacement.
        assert!(!seen.iter().any(|e| matches!(e, Event::Track(_))));
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !events
                .try_iter()
                .any(|e| e == Event::State(PlaybackState::Playing)),
            "doit rester en pause"
        );

        // Reprise : il ne reste que 0,5 s à jouer.
        let start = Instant::now();
        engine.send(Command::Play);
        wait_for(&events, |e| *e == Event::State(PlaybackState::Ended));
        assert!(
            start.elapsed() < Duration::from_millis(1_400),
            "{:?}",
            start.elapsed()
        );

        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        std::fs::remove_file(path).ok();
    }

    /// Joue `first` avec `second` en suivant ; renvoie les événements jusqu'à la fin.
    fn play_two(crossfade: Duration, allow_fade: bool) -> (Vec<Event>, Duration) {
        let first = wav_file(16_000, 8_000); // 2 s
        let second = wav_file(8_001, 8_000); // 1 s
        let (engine, _rt, stop) = engine();
        let events = engine.events();
        engine.send(Command::SetCrossfade(crossfade));
        engine.send(Command::Load(SourceSpec::File(first.clone())));
        wait_for(&events, |e| matches!(e, Event::Track(_)));
        engine.send(Command::SetNext(Some(NextTrack {
            spec: SourceSpec::File(second.clone()),
            crossfade: allow_fade,
        })));
        let start = Instant::now();
        let seen = wait_for(&events, |e| *e == Event::State(PlaybackState::Ended));
        let elapsed = start.elapsed();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        std::fs::remove_file(first).ok();
        std::fs::remove_file(second).ok();
        (seen, elapsed)
    }

    #[test]
    fn chains_next_track_without_gap() {
        let (seen, elapsed) = play_two(Duration::ZERO, true);
        let advanced: Vec<_> = seen
            .iter()
            .filter_map(|e| match e {
                Event::Advanced(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(advanced.len(), 1, "{seen:?}");
        // Pas de nouveau chargement : ni Loading ni Idle entre les deux.
        let after: Vec<_> = seen
            .iter()
            .skip_while(|e| !matches!(e, Event::Advanced(_)))
            .collect();
        assert!(!after.contains(&&Event::State(PlaybackState::Loading)));
        // 2 s + 1 s, sans attente entre les deux.
        assert!(elapsed < Duration::from_millis(3_600), "{elapsed:?}");
    }

    #[test]
    fn crossfades_into_next_track() {
        let (seen, elapsed) = play_two(Duration::from_millis(800), true);
        assert!(seen.iter().any(|e| matches!(e, Event::Advanced(_))));
        // Le suivant démarre 0,8 s avant la fin du premier.
        assert!(elapsed < Duration::from_millis(2_900), "{elapsed:?}");

        // Fondu refusé (même album) : enchaînement simple, plus long.
        let (seen, elapsed) = play_two(Duration::from_millis(800), false);
        assert!(seen.iter().any(|e| matches!(e, Event::Advanced(_))));
        assert!(elapsed > Duration::from_millis(2_900), "{elapsed:?}");
    }

    #[test]
    fn replay_gain_is_limited_by_peak() {
        let rg = ReplayGain {
            track_gain: Some(-6.),
            track_peak: Some(0.5),
            album_gain: Some(6.),
            album_peak: Some(0.8),
        };
        assert_eq!(replay_gain_factor(Some(&rg), ReplayGainMode::Off), 1.);
        let track = replay_gain_factor(Some(&rg), ReplayGainMode::Track);
        assert!((track - 0.501).abs() < 0.01, "{track}");
        // +6 dB (×2) limité par la crête 0,8 : ×1,25.
        let album = replay_gain_factor(Some(&rg), ReplayGainMode::Album);
        assert!((album - 1.25).abs() < 0.01, "{album}");
        assert_eq!(replay_gain_factor(None, ReplayGainMode::Album), 1.);
        assert_eq!(extension(Some("audio/aacp")), "aac");
        assert_eq!(extension(Some("m4a")), "m4a");
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
