//! Lecture des tags avec lofty.

use std::path::Path;
use std::time::Duration;

use lofty::picture::PictureType;
use lofty::prelude::*;

/// Métadonnées extraites d'un fichier.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TrackMeta {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub duration: Duration,
    /// Pochette intégrée (de face de préférence), données brutes.
    pub picture: Option<Vec<u8>>,
}

fn clean(s: Option<std::borrow::Cow<'_, str>>) -> Option<String> {
    let s = s?.trim().trim_matches('\0').trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Lit les tags ; un fichier sans tags (ou illisible par lofty) garde son
/// nom de fichier comme titre.
pub(crate) fn read(path: &Path) -> TrackMeta {
    let fallback_title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let file = match lofty::read_from_path(path) {
        Ok(file) => file,
        Err(e) => {
            tracing::debug!(path = %path.display(), error = %e, "tags illisibles");
            return TrackMeta {
                title: fallback_title,
                ..Default::default()
            };
        }
    };
    let duration = file.properties().duration();
    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else {
        return TrackMeta {
            title: fallback_title,
            duration,
            ..Default::default()
        };
    };

    let picture = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())
        .map(|p| p.data().to_vec());

    TrackMeta {
        title: clean(tag.title()).unwrap_or(fallback_title),
        artist: clean(tag.artist()),
        album: clean(tag.album()),
        album_artist: clean(tag.get_string(ItemKey::AlbumArtist).map(Into::into)),
        track_no: tag.track().filter(|&n| n > 0),
        disc_no: tag.disk().filter(|&n| n > 0),
        year: tag.date().map(|d| u32::from(d.year)).filter(|&y| y > 0),
        genre: clean(tag.genre()),
        duration,
        picture,
    }
}
