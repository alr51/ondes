//! Modèle du lecteur côté UI : relaie les commandes au moteur audio et
//! reflète ses événements dans une entité GPUI observable.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;
use ondes_audio::{
    Analyzer, AudioEngine, Command, Event, NextTrack, PlaybackState, Recorded, SourceSpec,
    TrackInfo,
};
use ondes_library::Track;
use ondes_radio::{HistoryEntry, HistoryKind, RadioError, Songs, Station};

use crate::history::{self, HistoryModel};
use crate::library::LibraryModel;
use crate::queue::{Queue, Repeat};
use crate::radio::{Directory, RadioFranceApi};
use crate::runtime::Rt;
use crate::settings::{AudioSettings, LastSource, Settings};

/// Autour d'un morceau Radio France (secondes) : l'émission qui le contient
/// n'est pas annoncée, le morceau suivant étant sans doute imminent.
const BETWEEN_SONGS: i64 = 120;

/// Événement à signaler à l'utilisateur (notification dans la fenêtre).
pub enum PlayerNotice {
    /// Morceau de la file illisible, passé automatiquement.
    Skipped { title: String, reason: String },
    /// Enregistrement d'un direct terminé.
    Recorded(Recorded),
}

impl EventEmitter<PlayerNotice> for Player {}

pub struct Player {
    engine: Option<AudioEngine>,
    pub state: PlaybackState,
    pub track: Option<TrackInfo>,
    /// Station de l'annuaire ou des favoris en cours, si la lecture vient de là.
    pub station: Option<Station>,
    /// Titre en cours annoncé par la radio.
    pub now_playing: Option<String>,
    /// Libellé affiché pendant l'ouverture d'une nouvelle source.
    pub pending: Option<String>,
    pub position: Duration,
    pub error: Option<String>,
    pub reconnecting: Option<u32>,
    /// File d'attente des morceaux de la bibliothèque.
    pub queue: Queue,
    /// Source du lancement précédent, relancée par « Lecture ».
    pub resume: Option<LastSource>,
    /// Morceaux illisibles d'affilée (évite de boucler sur une file entière).
    failures_in_row: usize,
    volume: f32,
    /// Fichier de l'enregistrement en cours du direct.
    pub recording: Option<PathBuf>,
    analyzer: Option<Analyzer>,
    /// Fondu enchaîné aussi entre deux morceaux d'un même album.
    crossfade_same_album: bool,
    /// Le fichier en cours est déjà inscrit dans l'historique.
    logged: bool,
    /// Temps réellement écouté du fichier en cours (les sauts ne comptent pas).
    listened: Duration,
    /// Suivi des titres d'une station Radio France (pas d'ICY sur leurs flux).
    radio_france: Option<Task<()>>,
    _events: Task<()>,
}

impl Player {
    pub fn new(volume: f32, audio: &AudioSettings, cx: &mut Context<Self>) -> Self {
        let (engine, error) = match AudioEngine::start(Rt::handle(cx)) {
            Ok(engine) => (Some(engine), None),
            Err(e) => {
                tracing::error!("moteur audio indisponible : {e}");
                (None, Some(e.to_string()))
            }
        };

        let events_task = match &engine {
            Some(engine) => {
                engine.send(Command::SetVolume(volume));
                engine.send(Command::SetEqualizer(audio.equalizer()));
                engine.send(Command::SetReplayGain(audio.replay_gain_mode()));
                engine.send(Command::SetCrossfade(audio.crossfade()));
                let events = engine.events();
                cx.spawn(async move |this, cx| {
                    while let Ok(event) = events.recv_async().await {
                        if this
                            .update(cx, |this, cx| {
                                this.apply(event, cx);
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
            }
            None => Task::ready(()),
        };

        let analyzer = engine.as_ref().map(AudioEngine::analyzer);
        if let Some(a) = &analyzer {
            a.set_enabled(audio.visualizer);
        }
        Self {
            engine,
            analyzer,
            recording: None,
            crossfade_same_album: audio.crossfade_same_album,
            logged: false,
            listened: Duration::ZERO,
            radio_france: None,
            state: PlaybackState::Idle,
            track: None,
            station: None,
            now_playing: None,
            pending: None,
            position: Duration::ZERO,
            error,
            reconnecting: None,
            queue: Queue::default(),
            resume: None,
            failures_in_row: 0,
            volume,
            _events: events_task,
        }
    }

    fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::State(PlaybackState::Ended) if !self.queue.is_empty() => {
                self.state = PlaybackState::Ended;
                if self.queue.next(true).is_some() {
                    self.play_current(cx);
                }
            }
            Event::State(state) => {
                self.state = state;
                if state == PlaybackState::Playing {
                    self.failures_in_row = 0;
                }
                // Plus de titres à suivre quand le direct ne joue plus (en
                // pause, le flux est coupé) ; repris avec la lecture.
                match state {
                    PlaybackState::Idle | PlaybackState::Ended | PlaybackState::Paused => {
                        self.radio_france = None
                    }
                    PlaybackState::Playing if self.radio_france.is_none() => {
                        self.watch_radio_france(cx)
                    }
                    _ => {}
                }
                match state {
                    PlaybackState::Loading => self.error = None,
                    PlaybackState::Playing | PlaybackState::Idle => self.reconnecting = None,
                    _ => {}
                }
            }
            Event::NowPlaying(title) => self.announce(title, cx),
            Event::Track(track) => {
                self.now_playing = None;
                self.logged = false;
                self.listened = Duration::ZERO;
                self.track = Some(track);
                self.pending = None;
                self.position = Duration::ZERO;
                self.sync_next();
                self.watch_radio_france(cx);
            }
            // Le moteur a enchaîné le morceau préchargé : la file avance aussi.
            Event::Advanced(track) => {
                self.queue.next(true);
                if self
                    .queue
                    .current()
                    .is_none_or(|t| SourceSpec::File(t.path.clone()) != track.spec)
                {
                    tracing::warn!(title = %track.title, "file et moteur désynchronisés");
                }
                self.failures_in_row = 0;
                self.now_playing = None;
                self.logged = false;
                self.listened = Duration::ZERO;
                self.track = Some(track);
                self.pending = None;
                self.position = Duration::ZERO;
                self.sync_next();
            }
            Event::RecordingStarted(path) => self.recording = Some(path),
            Event::RecordingStopped(done) => {
                self.recording = None;
                cx.emit(PlayerNotice::Recorded(done));
            }
            Event::Position(pos) => {
                // Seule l'avance régulière de la lecture compte comme écoute ;
                // un saut (ou un retour) n'ajoute rien.
                if let Some(step) = pos.checked_sub(self.position)
                    && step <= Duration::from_secs(2)
                    && self.state == PlaybackState::Playing
                {
                    self.listened += step;
                }
                self.position = pos;
                self.log_file(cx);
            }
            Event::Reconnecting { attempt } => self.reconnecting = Some(attempt),
            // Morceau illisible dans une file : on le signale et on passe au suivant.
            Event::Error(reason) if !self.queue.is_empty() && self.track.is_none() => {
                let title = self
                    .queue
                    .current()
                    .map(|t| t.title.clone())
                    .unwrap_or_default();
                self.failures_in_row += 1;
                cx.emit(PlayerNotice::Skipped {
                    title,
                    reason: reason.clone(),
                });
                if self.failures_in_row < self.queue.len() && self.queue.next(false).is_some() {
                    self.play_current(cx);
                } else {
                    self.error = Some(reason);
                    self.pending = None;
                }
            }
            Event::Error(message) => {
                self.error = Some(message);
                self.pending = None;
            }
        }
    }

    /// Titre annoncé par la radio (ICY, ou API Radio France).
    fn announce(&mut self, title: String, cx: &mut Context<Self>) {
        if self.now_playing.as_deref() == Some(title.as_str()) {
            return;
        }
        self.log_radio(&title, cx);
        self.now_playing = Some(title);
        cx.notify();
    }

    /// Station Radio France en cours : son titre est demandé à leur API au
    /// début de lecture, puis à la fin prévue de chaque morceau ou émission.
    fn watch_radio_france(&mut self, cx: &mut Context<Self>) {
        self.radio_france = None;
        let Some(track) = self.track.as_ref().filter(|t| t.live) else {
            return;
        };
        let spec_url = match &track.spec {
            SourceSpec::Url(url) => Some(url.as_str()),
            SourceSpec::File(_) => None,
        };
        let station_url = self.station.as_ref().map(|s| s.url.as_str());
        let Some(id) = [spec_url, station_url]
            .into_iter()
            .flatten()
            .find_map(ondes_radio::radio_france_station)
        else {
            return;
        };
        let Some(api) = RadioFranceApi::get(cx) else {
            return;
        };
        tracing::info!(id, "titres Radio France");
        self.radio_france = Some(cx.spawn(async move |this, cx| {
            loop {
                let now = history::now();
                let api = api.clone();
                let Ok(task) = this.update(cx, |_, cx| {
                    Rt::spawn(cx, async move { api.on_air(id, now).await })
                }) else {
                    return;
                };
                let wait = match task.await {
                    Ok(Ok(Some(on_air))) => {
                        let now = history::now();
                        // Juste avant ou après un morceau, l'émission ne
                        // s'affiche pas : le morceau suivant arrive sans doute.
                        let soon = match on_air.songs {
                            Songs::Ended(end) if now - end < BETWEEN_SONGS => Some(20),
                            Songs::Next(start) if start - now < BETWEEN_SONGS => {
                                Some((start - now + 3).clamp(5, BETWEEN_SONGS))
                            }
                            _ => None,
                        };
                        if let Some(wait) = soon {
                            wait
                        } else {
                            let end = on_air.end;
                            let songs = on_air.songs;
                            if this
                                .update(cx, |p, cx| p.announce(on_air.text, cx))
                                .is_err()
                            {
                                return;
                            }
                            // Un peu après la fin prévue, pour laisser l'API changer.
                            let wait = end.map_or(60, |end| (end - now + 5).clamp(15, 180));
                            // Une émission qui diffuse des morceaux : le
                            // prochain peut commencer à tout moment.
                            if songs == Songs::None {
                                wait
                            } else {
                                wait.min(60)
                            }
                        }
                    }
                    Ok(Ok(None)) => 30,
                    Ok(Err(e)) => {
                        tracing::warn!(id, "titres Radio France : {e}");
                        // Refus du serveur : on se fait oublier un moment.
                        match e {
                            RadioError::Status(400..500) => 600,
                            _ => 60,
                        }
                    }
                    Err(_) => return,
                };
                cx.background_executor()
                    .timer(Duration::from_secs(wait as u64))
                    .await;
            }
        }));
    }

    /// Inscrit dans l'historique le titre annoncé par la radio en cours.
    fn log_radio(&self, title: &str, cx: &mut Context<Self>) {
        if title.trim().is_empty() {
            return;
        }
        let (name, url, favicon) = match (&self.station, &self.track) {
            (Some(s), _) => (s.name.clone(), s.url.clone(), s.favicon.clone()),
            (None, Some(t)) if t.live => match &t.spec {
                SourceSpec::Url(url) => (t.title.clone(), url.clone(), None),
                SourceSpec::File(_) => return,
            },
            _ => return,
        };
        let entry = HistoryEntry::radio(history::now(), title, &name, &url, favicon);
        HistoryModel::global(cx).update(cx, |h, cx| h.record(entry, cx));
    }

    /// Inscrit le fichier en cours une fois écouté assez longtemps.
    fn log_file(&mut self, cx: &mut Context<Self>) {
        let Some(track) = self.track.as_ref().filter(|t| !t.live) else {
            return;
        };
        let SourceSpec::File(path) = &track.spec else {
            return;
        };
        let needed = track
            .duration
            .map_or(history::LISTENED, |d| d.div_f32(2.).min(history::LISTENED));
        if self.logged || self.listened < needed {
            return;
        }
        self.logged = true;
        // Tags de la file, ou de la bibliothèque pour un fichier ouvert directement.
        let tags = self
            .current_track()
            .filter(|t| &t.path == path)
            .cloned()
            .or_else(|| {
                LibraryModel::global(cx)
                    .read(cx)
                    .data
                    .tracks
                    .iter()
                    .find(|t| &t.path == path)
                    .cloned()
            });
        let tags = tags.as_ref();
        let entry = HistoryEntry {
            id: 0,
            played_at: history::now(),
            kind: HistoryKind::File,
            title: tags.map_or_else(|| track.title.clone(), |t| t.title.clone()),
            artist: tags.and_then(|t| t.artist.clone()),
            source: tags.and_then(|t| t.album.clone()),
            location: path.to_string_lossy().into_owned(),
            favicon: None,
        };
        HistoryModel::global(cx).update(cx, |h, cx| h.record(entry, cx));
    }

    fn send(&self, cmd: Command) {
        if let Some(engine) = &self.engine {
            engine.send(cmd);
        }
    }

    pub fn load(&mut self, spec: SourceSpec, cx: &mut Context<Self>) {
        self.station = None;
        self.queue.clear();
        self.start(spec.label(), spec, cx);
    }

    pub fn play_station(&mut self, station: Station, cx: &mut Context<Self>) {
        Directory::count_click(&station, cx);
        let spec = SourceSpec::Url(station.url.clone());
        let label = station.name.clone();
        self.station = Some(station);
        self.queue.clear();
        self.start(label, spec, cx);
    }

    fn start(&mut self, label: String, spec: SourceSpec, cx: &mut Context<Self>) {
        self.resume = None;
        self.radio_france = None;
        self.now_playing = None;
        self.pending = Some(label);
        self.track = None;
        self.error = None;
        self.position = Duration::ZERO;
        self.send(Command::Load(spec));
        cx.notify();
    }

    /// Joue `tracks` (la liste visible) à partir de `start`.
    pub fn play_tracks(&mut self, tracks: Vec<Track>, start: usize, cx: &mut Context<Self>) {
        self.queue.set(tracks, start);
        self.play_current(cx);
    }

    fn play_current(&mut self, cx: &mut Context<Self>) {
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        self.station = None;
        self.start(track.title.clone(), SourceSpec::File(track.path), cx);
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        if self.queue.next(false).is_some() {
            self.play_current(cx);
        }
    }

    /// Au-delà de 3 s, « précédent » revient au début du morceau.
    pub fn prev(&mut self, cx: &mut Context<Self>) {
        if self.position > Duration::from_secs(3) {
            self.seek(Duration::ZERO, cx);
        } else if self.queue.prev().is_some() {
            self.play_current(cx);
        }
    }

    /// Déplacement relatif (millisecondes, négatif pour reculer).
    pub fn seek_by(&mut self, delta_ms: i64, cx: &mut Context<Self>) {
        let Some(duration) = self
            .track
            .as_ref()
            .filter(|t| !t.live)
            .and_then(|t| t.duration)
        else {
            return;
        };
        let target =
            (self.position.as_millis() as i64 + delta_ms).clamp(0, duration.as_millis() as i64);
        self.seek(Duration::from_millis(target as u64), cx);
    }

    pub fn change_volume(&mut self, delta: f32, cx: &mut Context<Self>) {
        let v = (self.volume + delta).clamp(0., 1.);
        self.set_volume(v, cx);
    }

    /// Source en cours, à mémoriser pour le prochain lancement.
    pub fn last_source(&self) -> Option<LastSource> {
        if let Some(t) = self.current_track() {
            return Some(LastSource::Track {
                path: t.path.clone(),
            });
        }
        if let Some(station) = &self.station {
            return Some(LastSource::Station {
                station: Box::new(station.clone()),
            });
        }
        match self.track.as_ref().map(|t| &t.spec) {
            Some(SourceSpec::Url(url)) => Some(LastSource::Url { url: url.clone() }),
            Some(SourceSpec::File(path)) => Some(LastSource::File { path: path.clone() }),
            None => self.resume.clone(),
        }
    }

    /// Relance la source du lancement précédent.
    fn play_resume(&mut self, cx: &mut Context<Self>) {
        let Some(last) = self.resume.take() else {
            return;
        };
        match last {
            LastSource::Station { station } => self.play_station(*station, cx),
            LastSource::Url { url } => self.load(SourceSpec::Url(url), cx),
            LastSource::File { path } => self.load(SourceSpec::File(path), cx),
            LastSource::Track { path } => {
                // Avec ses tags si le morceau est (encore) dans la bibliothèque.
                let library = LibraryModel::global(cx);
                let found = library
                    .read(cx)
                    .data
                    .tracks
                    .iter()
                    .find(|t| t.path == path)
                    .cloned();
                match found {
                    Some(track) => self.play_tracks(vec![track], 0, cx),
                    None => self.load(SourceSpec::File(path), cx),
                }
            }
        }
    }

    pub fn seek(&mut self, pos: Duration, cx: &mut Context<Self>) {
        self.position = pos;
        self.send(Command::Seek(pos));
        cx.notify();
    }

    pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        self.queue.set_shuffle(!self.queue.shuffle);
        self.sync_next();
        cx.notify();
    }

    pub fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        self.queue.repeat = self.queue.repeat.cycle();
        self.sync_next();
        cx.notify();
    }

    /// Indique au moteur le morceau à précharger et à enchaîner.
    fn sync_next(&self) {
        let next = self
            .queue
            .current()
            .zip(self.queue.peek_next())
            .map(|(cur, next)| {
                let same_album = cur.album.is_some()
                    && cur.album == next.album
                    && cur.album_artist_or_artist() == next.album_artist_or_artist();
                NextTrack {
                    spec: SourceSpec::File(next.path.clone()),
                    crossfade: self.crossfade_same_album || !same_album,
                }
            });
        self.send(Command::SetNext(next));
    }

    /// Applique les réglages audio (égaliseur, normalisation, fondu, spectre).
    pub fn apply_audio_settings(&mut self, audio: &AudioSettings) {
        self.send(Command::SetEqualizer(audio.equalizer()));
        self.send(Command::SetReplayGain(audio.replay_gain_mode()));
        self.send(Command::SetCrossfade(audio.crossfade()));
        if let Some(a) = &self.analyzer {
            a.set_enabled(audio.visualizer);
        }
        if self.crossfade_same_album != audio.crossfade_same_album {
            self.crossfade_same_album = audio.crossfade_same_album;
            self.sync_next();
        }
    }

    pub fn analyzer(&self) -> Option<&Analyzer> {
        self.analyzer.as_ref()
    }

    /// Le direct en cours peut-il être enregistré ?
    pub fn can_record(&self) -> bool {
        self.track.as_ref().is_some_and(|t| t.live) && self.state.is_active()
    }

    /// Commence ou arrête l'enregistrement du direct.
    pub fn toggle_recording(&mut self, cx: &mut Context<Self>) {
        if self.recording.is_some() {
            self.send(Command::Record(None));
            return;
        }
        if !self.can_record() {
            return;
        }
        let name = self
            .current_station()
            .map(|s| s.name)
            .or_else(|| self.title())
            .unwrap_or_else(|| "Radio".into());
        let dir = Settings::get(cx).audio.record_dir();
        let stamp = chrono::Local::now().format("%Y-%m-%d %Hh%M");
        self.send(Command::Record(Some(
            dir.join(format!("{} {stamp}", file_name_safe(&name))),
        )));
    }

    pub fn repeat(&self) -> Repeat {
        self.queue.repeat
    }

    /// Morceau de la bibliothèque en cours.
    pub fn current_track(&self) -> Option<&Track> {
        self.queue.current()
    }

    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if self.track.is_none() && self.pending.is_none() && self.resume.is_some() {
            self.play_resume(cx);
            return;
        }
        self.send(Command::TogglePlay);
    }

    pub fn stop(&mut self, _: &mut Context<Self>) {
        self.send(Command::Stop);
    }

    pub fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        self.volume = volume;
        self.send(Command::SetVolume(volume));
        cx.notify();
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Titre principal à afficher.
    pub fn title(&self) -> Option<String> {
        self.current_track()
            .map(|t| t.title.clone())
            .or_else(|| self.station.as_ref().map(|s| s.name.clone()))
            .or_else(|| self.track.as_ref().map(|t| t.title.clone()))
            .or_else(|| self.pending.clone())
            .or_else(|| {
                self.resume.as_ref().map(|r| match r {
                    LastSource::Station { station } => station.name.clone(),
                    LastSource::Url { url } => SourceSpec::Url(url.clone()).label(),
                    LastSource::File { path } | LastSource::Track { path } => {
                        SourceSpec::File(path.clone()).label()
                    }
                })
            })
    }

    /// La radio en cours, sous forme de station (pour l'ajouter aux favoris).
    pub fn current_station(&self) -> Option<Station> {
        if let Some(station) = &self.station {
            return Some(station.clone());
        }
        let track = self.track.as_ref().filter(|t| t.live)?;
        match &track.spec {
            SourceSpec::Url(url) => Some(Station::manual(track.title.clone(), url.clone())),
            SourceSpec::File(_) => None,
        }
    }

    /// « Artiste — Album » du morceau de la bibliothèque en cours.
    pub fn track_subtitle(&self) -> Option<String> {
        let t = self.current_track()?;
        let parts: Vec<&str> = [t.artist.as_deref(), t.album.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        (!parts.is_empty()).then(|| parts.join(" — "))
    }

    /// Y a-t-il quelque chose à lire ou à arrêter ?
    pub fn has_source(&self) -> bool {
        self.engine.is_some()
            && (self.track.is_some() || self.pending.is_some() || self.resume.is_some())
    }
}

/// Nom utilisable comme nom de fichier sur tous les systèmes.
fn file_name_safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    let cleaned: String = cleaned.chars().take(80).collect();
    if cleaned.is_empty() {
        "Radio".into()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn safe_file_names() {
        assert_eq!(
            super::file_name_safe("FIP / Jazz: live?"),
            "FIP - Jazz- live-"
        );
        assert_eq!(super::file_name_safe(" ..  "), "Radio");
    }
}
