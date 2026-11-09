//! État persistant entre deux lancements : `~/.local/share/ondes/state.toml`.
//!
//! Volume, dernière source écoutée, page, thème, fenêtre, aléatoire et
//! répétition, notifications, mises à jour, réglages audio. Écrit avec un léger délai après chaque
//! changement (et de façon atomique) pour ne pas solliciter le disque.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;
use ondes_radio::Station;
use serde::{Deserialize, Serialize};

const SAVE_DELAY: Duration = Duration::from_millis(800);

/// Dernière source écoutée, proposée à la relecture au démarrage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LastSource {
    Station {
        station: Box<Station>,
    },
    Url {
        url: String,
    },
    File {
        path: PathBuf,
    },
    /// Morceau de la bibliothèque (retrouvé dans l'index pour ses tags).
    Track {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
}

/// Réglages audio : égaliseur, normalisation, enchaînements, spectre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub eq_enabled: bool,
    /// Correction des 10 bandes, en dB.
    pub eq_gains: [f32; 10],
    /// `off`, `track` ou `album`.
    pub replay_gain: String,
    /// Durée du fondu enchaîné, en secondes (0 : enchaînement sans blanc).
    pub crossfade_secs: f32,
    /// Fondu aussi entre deux morceaux d'un même album.
    pub crossfade_same_album: bool,
    /// Spectre dans la barre de lecture.
    pub visualizer: bool,
    /// Dossier des enregistrements de flux (défaut : `Musique/Ondes`).
    pub record_dir: Option<PathBuf>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            eq_enabled: false,
            eq_gains: [0.; 10],
            replay_gain: "off".into(),
            crossfade_secs: 0.,
            crossfade_same_album: false,
            visualizer: true,
            record_dir: None,
        }
    }
}

impl AudioSettings {
    pub fn equalizer(&self) -> ondes_audio::Equalizer {
        ondes_audio::Equalizer {
            enabled: self.eq_enabled,
            gains: self.eq_gains,
        }
    }

    pub fn replay_gain_mode(&self) -> ondes_audio::ReplayGainMode {
        match self.replay_gain.as_str() {
            "track" => ondes_audio::ReplayGainMode::Track,
            "album" => ondes_audio::ReplayGainMode::Album,
            _ => ondes_audio::ReplayGainMode::Off,
        }
    }

    pub fn crossfade(&self) -> Duration {
        Duration::from_secs_f32(self.crossfade_secs.clamp(0., 15.))
    }

    pub fn record_dir(&self) -> PathBuf {
        self.record_dir
            .clone()
            .unwrap_or_else(crate::paths::recordings)
    }
}

/// Vue compacte du lecteur (`window` : taille de la fenêtre dans cette vue).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactState {
    /// `cover`, `spectrum`, `title` ou `queue`.
    pub mode: String,
    pub window: Option<WindowState>,
}

impl Default for CompactState {
    fn default() -> Self {
        Self {
            mode: "cover".into(),
            window: None,
        }
    }
}

/// Visualiseurs par shaders.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VisualState {
    /// Effet choisi (indice dans `ondes_visual::EFFECTS`).
    pub effect: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub volume: f32,
    /// `None` : suit le thème du système.
    pub dark: Option<bool>,
    pub page: String,
    pub shuffle: bool,
    pub repeat: String,
    pub notifications: bool,
    /// Historique d'écoute : titres des radios et morceaux joués.
    pub history: bool,
    /// Vérifie au démarrage si une version plus récente est publiée.
    pub check_updates: bool,
    /// Dernière vérification réussie (secondes Unix).
    pub last_update_check: Option<u64>,
    pub window: Option<WindowState>,
    pub last: Option<LastSource>,
    pub audio: AudioSettings,
    pub compact: CompactState,
    pub visual: VisualState,
    /// Mode d'affichage de la page « En cours ».
    pub now_playing_mode: String,
    /// Barre latérale pliée (icônes seules).
    pub sidebar_collapsed: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            volume: 0.8,
            dark: None,
            page: "home".into(),
            shuffle: false,
            repeat: "off".into(),
            notifications: true,
            history: true,
            check_updates: true,
            last_update_check: None,
            window: None,
            last: None,
            audio: AudioSettings::default(),
            compact: CompactState::default(),
            visual: VisualState::default(),
            now_playing_mode: "cover".into(),
            sidebar_collapsed: false,
        }
    }
}

impl AppState {
    pub fn load() -> Self {
        let path = crate::paths::state_file();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!(path = %path.display(), "état illisible, valeurs par défaut : {e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    fn save(&self) -> std::io::Result<()> {
        let path = crate::paths::state_file();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    }
}

/// État partagé, enregistré après chaque modification.
pub struct Settings {
    pub state: AppState,
    _save: Option<Task<()>>,
}

pub struct GlobalSettings(pub Entity<Settings>);
impl Global for GlobalSettings {}

impl Settings {
    pub fn init(state: AppState, cx: &mut App) {
        let entity = cx.new(|_| Self { state, _save: None });
        cx.set_global(GlobalSettings(entity));
    }

    pub fn global(cx: &App) -> Entity<Settings> {
        cx.global::<GlobalSettings>().0.clone()
    }

    pub fn get(cx: &App) -> AppState {
        Self::global(cx).read(cx).state.clone()
    }

    /// Modifie l'état ; l'écriture sur disque est regroupée.
    pub fn update(cx: &mut App, f: impl FnOnce(&mut AppState)) {
        Self::global(cx).update(cx, |this, cx| {
            let before = this.state.clone();
            f(&mut this.state);
            if this.state == before {
                return;
            }
            cx.notify();
            this._save = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SAVE_DELAY).await;
                this.update(cx, |this, _| this.save_now()).ok();
            }));
        });
    }

    pub fn save_now(&mut self) {
        self._save = None;
        if let Err(e) = self.state.save() {
            tracing::warn!("enregistrement de l'état impossible : {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppState, LastSource, WindowState};

    #[test]
    fn round_trips_and_tolerates_missing_fields() {
        let state = AppState {
            volume: 0.42,
            dark: Some(true),
            last: Some(LastSource::Station {
                station: Box::new(ondes_radio::Station::manual(
                    "FIP",
                    "https://icecast.radiofrance.fr/fip-midfi.mp3",
                )),
            }),
            window: Some(WindowState {
                x: 10.,
                y: 20.,
                width: 1100.,
                height: 720.,
                maximized: false,
            }),
            ..Default::default()
        };
        let text = toml::to_string_pretty(&state).unwrap();
        assert_eq!(toml::from_str::<AppState>(&text).unwrap(), state);

        // Fichier d'une version précédente : champs manquants = défauts.
        let old: AppState = toml::from_str("volume = 0.5\n").unwrap();
        assert_eq!(old.volume, 0.5);
        assert!(old.notifications);
        assert!(old.check_updates);
        assert_eq!(old.last, None);
        assert_eq!(old.audio, super::AudioSettings::default());
    }
}
