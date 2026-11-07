//! Emplacements des données (base, cache) selon la plateforme.

use std::path::PathBuf;

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", "ondes")
}

/// Base SQLite (favoris, et plus tard bibliothèque) : `~/.local/share/ondes/ondes.db`.
pub fn database() -> PathBuf {
    project_dirs()
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("ondes"))
        .join("ondes.db")
}

/// Cache des logos de stations : `~/.cache/ondes/logos`.
pub fn logo_cache() -> PathBuf {
    project_dirs()
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("ondes-cache"))
        .join("logos")
}

/// Miniatures de pochettes : `~/.cache/ondes/covers`.
pub fn cover_cache() -> PathBuf {
    project_dirs()
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("ondes-cache"))
        .join("covers")
}

/// Dossier Musique de l'utilisateur, proposé au premier lancement.
pub fn music_dir() -> Option<PathBuf> {
    directories::UserDirs::new()
        .and_then(|d| d.audio_dir().map(|p| p.to_path_buf()))
        .filter(|p| p.is_dir())
}

/// État persistant (volume, dernière source, fenêtre…) : `~/.local/share/ondes/state.toml`.
pub fn state_file() -> PathBuf {
    database().with_file_name("state.toml")
}

/// Journal de la session : `~/.local/share/ondes/ondes.log` (le précédent en `.1`).
pub fn log_file() -> PathBuf {
    database().with_file_name("ondes.log")
}
