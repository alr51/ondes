//! Sources audio d'Ondes.
//!
//! Une source ouverte ([`Opened`]) fournit un flux d'octets `Read + Seek`
//! prêt à être décodé, plus quelques indications (format, titre, direct ou
//! non). Le décodage lui-même est fait par `ondes-audio`.

mod hls;
mod http;
mod icy;
mod local;
mod playlist;
mod ts;

use std::io::{Read, Seek};
use std::path::PathBuf;

pub use http::{HttpOptions, StreamStatus, open_stream};
pub use icy::{IcyDemuxer, parse_stream_title};
pub use local::open_file;
pub use playlist::{PlaylistEntry, PlaylistKind, first_stream_url, parse_entries};

/// Ce que l'utilisateur demande à écouter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSpec {
    File(PathBuf),
    Url(String),
}

impl SourceSpec {
    /// Interprète un texte saisi : URL http(s) ou chemin local.
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }
        if input.starts_with("http://") || input.starts_with("https://") {
            return Some(Self::Url(input.to_string()));
        }
        let path = PathBuf::from(input.strip_prefix("file://").unwrap_or(input));
        // Chemin absolu : la source reste valable quel que soit le dossier courant.
        let path = match std::path::absolute(&path) {
            Ok(abs) => abs,
            Err(_) => path,
        };
        Some(Self::File(path))
    }

    /// Libellé provisoire, avant ouverture.
    pub fn label(&self) -> String {
        match self {
            Self::File(path) => local::title_from_path(path),
            Self::Url(url) => http::host_of(url).unwrap_or_else(|| url.clone()),
        }
    }
}

/// Flux d'octets décodable.
pub trait MediaReader: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> MediaReader for T {}

/// Source prête à être décodée.
pub struct Opened {
    pub reader: Box<dyn MediaReader>,
    /// Titre affichable (nom de station, nom de fichier).
    pub title: String,
    /// Extension indicative pour le décodeur (`mp3`, `aac`, `ogg`…).
    pub hint: Option<String>,
    /// Type MIME annoncé par le serveur.
    pub mime: Option<String>,
    /// Taille totale si connue.
    pub byte_len: Option<u64>,
    /// `true` pour un flux sans fin (radio) : ni seek ni durée.
    pub live: bool,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("title", &self.title)
            .field("hint", &self.hint)
            .field("mime", &self.mime)
            .field("byte_len", &self.byte_len)
            .field("live", &self.live)
            .finish_non_exhaustive()
    }
}

/// Message lisible pour un code HTTP d'erreur.
fn http_status_message(code: u16) -> String {
    match code {
        404 | 410 => {
            format!("flux introuvable ({code}) : la station a sans doute changé d'adresse")
        }
        401 | 403 => format!("accès refusé par le serveur ({code})"),
        429 => "le serveur limite les connexions (429) : réessaie dans un moment".into(),
        500..=599 => format!("le serveur de la station est en panne ({code})"),
        _ => format!("le serveur a répondu {code}"),
    }
}

/// Erreurs d'ouverture d'une source.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("fichier illisible : {0}")]
    Io(#[from] std::io::Error),
    #[error("URL invalide : {0}")]
    InvalidUrl(String),
    #[error("erreur réseau : {0}")]
    Network(String),
    #[error("{}", http_status_message(*.0))]
    HttpStatus(u16),
    #[error("ce n'est pas un flux audio (type « {0} »)")]
    NotAudio(String),
    #[error("playlist vide ou illisible")]
    EmptyPlaylist,
    #[error("trop de redirections de playlist")]
    PlaylistLoop,
    #[error("playlist HLS : c'est un flux, à ajouter par son URL")]
    HlsPlaylist,
    #[error("flux HLS : {0}")]
    Hls(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_spec() {
        assert_eq!(
            SourceSpec::parse(" https://icecast.radiofrance.fr/fip-midfi.mp3 "),
            Some(SourceSpec::Url(
                "https://icecast.radiofrance.fr/fip-midfi.mp3".into()
            ))
        );
        assert_eq!(
            SourceSpec::parse("file:///tmp/a.mp3"),
            Some(SourceSpec::File("/tmp/a.mp3".into()))
        );
        assert_eq!(SourceSpec::parse("   "), None);
        assert!(
            SourceError::HttpStatus(404)
                .to_string()
                .contains("changé d'adresse")
        );
        assert!(
            SourceError::HttpStatus(503)
                .to_string()
                .contains("en panne")
        );
        assert_eq!(
            SourceSpec::parse("musique/a.mp3"),
            Some(SourceSpec::File(
                std::env::current_dir().unwrap().join("musique/a.mp3")
            ))
        );
    }
}
