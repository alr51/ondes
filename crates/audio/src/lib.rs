//! Moteur audio d'Ondes.
//!
//! Le moteur vit sur son propre thread. L'UI lui envoie des [`Command`] et
//! reçoit des [`Event`] par des canaux `flume` (utilisables en synchrone ou
//! en async, quel que soit l'exécuteur).
//!
//! Pipeline d'une lecture :
//! `source (fichier / HTTP)` → thread de décodage (symphonia via rodio) →
//! file d'échantillons bornée → [`source::StreamingSource`] tirée par la
//! sortie audio. La sortie ne bloque jamais : si la file est vide, elle joue
//! du silence et l'état passe à « mise en tampon ».
//!
//! Au moment de partir vers la sortie, les échantillons passent par
//! l'égaliseur puis par un gain lissé (volume, ReplayGain, fondus). Le
//! morceau suivant d'une file est préchargé : il s'enchaîne sans blanc, ou en
//! fondu enchaîné.

mod dsp;
mod engine;
mod source;

use std::path::PathBuf;
use std::time::Duration;

pub use dsp::{Analyzer, AnalyzerHold, EQ_FREQS, EQ_MAX_DB, Equalizer};
pub use engine::AudioEngine;
pub use ondes_sources::{Recorded, ReplayGain, SourceSpec};

/// Normalisation du volume d'après les tags ReplayGain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplayGainMode {
    #[default]
    Off,
    /// Gain propre à chaque morceau.
    Track,
    /// Gain de l'album : garde les écarts voulus entre ses morceaux.
    Album,
}

/// Morceau à enchaîner après celui en cours.
#[derive(Debug, Clone, PartialEq)]
pub struct NextTrack {
    pub spec: SourceSpec,
    /// Fondu enchaîné permis (faux entre deux morceaux d'un même album).
    pub crossfade: bool,
}

/// Ordres envoyés au moteur.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Charge et lance une source (remplace la lecture en cours).
    Load(SourceSpec),
    /// Reprend la lecture (ou relance la dernière source après un arrêt).
    Play,
    Pause,
    /// Bascule lecture / pause.
    TogglePlay,
    Stop,
    /// Volume entre 0 et 1 (échelle perceptive appliquée par le moteur).
    SetVolume(f32),
    /// Se place à cette position (fichiers seulement ; ignoré pour un direct).
    Seek(Duration),
    SetEqualizer(Equalizer),
    SetReplayGain(ReplayGainMode),
    /// Durée du fondu enchaîné (zéro : enchaînement sans blanc).
    SetCrossfade(Duration),
    /// Morceau à précharger et à enchaîner à la fin de celui en cours.
    SetNext(Option<NextTrack>),
    /// Enregistre le direct en cours dans ce fichier (extension ajoutée
    /// d'après le format), ou arrête l'enregistrement.
    Record(Option<PathBuf>),
}

/// État de lecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackState {
    #[default]
    Idle,
    /// Ouverture de la source et détection du format.
    Loading,
    /// Lecture lancée mais tampon insuffisant : on attend des données.
    Buffering,
    Playing,
    Paused,
    /// Fin du morceau ou du flux.
    Ended,
}

impl PlaybackState {
    /// La source est-elle « active » (le bouton doit proposer Pause) ?
    pub fn is_active(self) -> bool {
        matches!(self, Self::Loading | Self::Buffering | Self::Playing)
    }
}

/// Informations sur la source en cours.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    pub spec: SourceSpec,
    pub title: String,
    /// Flux en direct : pas de durée, pause = arrêt + reconnexion.
    pub live: bool,
    pub duration: Option<Duration>,
    /// Format tel qu'annoncé (`mp3`, `aac`, type MIME…).
    pub format: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    pub replay_gain: Option<ReplayGain>,
}

/// Événements émis par le moteur.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    State(PlaybackState),
    /// Nouvelle source chargée.
    Track(TrackInfo),
    /// Passage automatique au morceau préchargé ([`Command::SetNext`]).
    Advanced(TrackInfo),
    RecordingStarted(PathBuf),
    /// Enregistrement terminé (arrêt demandé, changement de source, erreur).
    RecordingStopped(Recorded),
    /// Position de lecture (émise régulièrement pendant la lecture).
    Position(Duration),
    /// Titre en cours annoncé par la radio (ICY), au moment où il devient audible.
    NowPlaying(String),
    /// Flux coupé, reconnexion en cours.
    Reconnecting {
        attempt: u32,
    },
    /// Erreur à afficher ; l'état est émis séparément.
    Error(String),
}

/// Erreurs de démarrage du moteur.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("aucune sortie audio disponible : {0}")]
    NoOutput(String),
    #[error("impossible de démarrer le thread audio : {0}")]
    Thread(#[from] std::io::Error),
}
