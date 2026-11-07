//! Webradios : annuaire Radio Browser, favoris (SQLite), import de playlists.

mod browser;
mod favorites;
mod import;
mod station;

pub use browser::{Country, Language, RadioBrowser, SearchQuery, Tag};
pub use favorites::Favorites;
pub use import::import_playlist;
pub use station::Station;

/// Erreurs de la crate.
#[derive(Debug, thiserror::Error)]
pub enum RadioError {
    #[error("annuaire Radio Browser injoignable : {0}")]
    Network(String),
    #[error("réponse inattendue de Radio Browser : {0}")]
    Decode(String),
    #[error("base de données : {0}")]
    Db(#[from] rusqlite::Error),
    #[error("fichier illisible : {0}")]
    Io(#[from] std::io::Error),
    #[error("playlist : {0}")]
    Playlist(#[from] ondes_sources::SourceError),
    #[error("ce fichier n'est pas une playlist .m3u ou .pls")]
    NotAPlaylist,
}
