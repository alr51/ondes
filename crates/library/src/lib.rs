//! Bibliothèque musicale locale.
//!
//! - [`scan`] parcourt les dossiers, lit les tags des fichiers nouveaux ou
//!   modifiés seulement, extrait les pochettes et met à jour l'index SQLite ;
//! - [`LibraryDb`] gère les dossiers et charge l'index ;
//! - [`model`] regroupe les morceaux en albums et artistes ;
//! - [`search`] filtre en mémoire, sans tenir compte des accents ni de la casse.
//!
//! Toute la bibliothèque tient en mémoire (quelques Mo pour 50 000
//! morceaux) : les vues filtrent et trient sans aller-retour SQL.

mod covers;
mod db;
pub mod model;
mod scan;
pub mod search;
mod tags;

use std::path::PathBuf;
use std::time::Duration;

pub use db::LibraryDb;
pub use model::{Album, Artist, albums, artists};
pub use scan::{ScanOptions, ScanProgress, ScanReport, is_audio_file, scan};

/// Un morceau indexé.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Track {
    pub path: PathBuf,
    /// Titre des tags, ou nom du fichier.
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub duration: Duration,
    /// Miniature de pochette (fichier du cache).
    pub cover: Option<PathBuf>,
}

impl Track {
    /// Artiste de l'album, à défaut celui du morceau.
    pub fn album_artist_or_artist(&self) -> Option<&str> {
        self.album_artist.as_deref().or(self.artist.as_deref())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("base de données : {0}")]
    Db(#[from] rusqlite::Error),
    #[error("fichier : {0}")]
    Io(#[from] std::io::Error),
    #[error("scan annulé")]
    Cancelled,
}
