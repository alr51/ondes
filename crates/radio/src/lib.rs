//! Webradios : annuaire Radio Browser, favoris et historique d'écoute
//! (SQLite), import de playlists, titres des stations Radio France.

mod browser;
mod favorites;
mod history;
mod import;
mod radiofrance;
mod station;

pub use browser::{Country, Language, RadioBrowser, SearchQuery, Tag};
pub use favorites::Favorites;
pub use history::{History, HistoryEntry, HistoryKind};
pub use import::import_playlist;
pub use radiofrance::{OnAir, RadioFrance, Songs, station_id as radio_france_station};
pub use station::Station;

/// Erreurs de la crate.
#[derive(Debug, thiserror::Error)]
pub enum RadioError {
    #[error("annuaire Radio Browser injoignable : {0}")]
    Network(String),
    #[error("réponse inattendue de Radio Browser : {0}")]
    Decode(String),
    /// Refus du serveur (titres Radio France).
    #[error("réponse HTTP {0}")]
    Status(u16),
    #[error("base de données : {0}")]
    Db(#[from] rusqlite::Error),
    #[error("fichier illisible : {0}")]
    Io(#[from] std::io::Error),
    #[error("playlist : {0}")]
    Playlist(#[from] ondes_sources::SourceError),
    #[error("ce fichier n'est pas une playlist .m3u ou .pls")]
    NotAPlaylist,
}
