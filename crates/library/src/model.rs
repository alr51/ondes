//! Regroupements : albums et artistes, à partir de la liste des morceaux.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::Track;
use crate::search::fold;

/// Album : morceaux partageant (artiste de l'album, titre de l'album).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Album {
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<u32>,
    pub cover: Option<PathBuf>,
    /// Indices dans la liste de morceaux, dans l'ordre disque / piste.
    pub tracks: Vec<usize>,
    pub duration: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artist {
    pub name: String,
    pub albums: usize,
    /// Indices dans la liste de morceaux, par album puis piste.
    pub tracks: Vec<usize>,
}

/// Ordre « naturel » dans un album : disque, piste, puis titre.
pub fn album_order(a: &Track, b: &Track) -> Ordering {
    (
        a.disc_no.unwrap_or(1),
        a.track_no.unwrap_or(u32::MAX),
        &a.title,
    )
        .cmp(&(
            b.disc_no.unwrap_or(1),
            b.track_no.unwrap_or(u32::MAX),
            &b.title,
        ))
}

/// Albums triés par artiste puis titre (les morceaux sans album sont exclus).
pub fn albums(tracks: &[Track]) -> Vec<Album> {
    let mut groups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (ix, t) in tracks.iter().enumerate() {
        let Some(album) = &t.album else { continue };
        let artist = t.album_artist_or_artist().unwrap_or_default();
        groups
            .entry((fold(artist), fold(album)))
            .or_default()
            .push(ix);
    }
    groups
        .into_values()
        .map(|mut ixs| {
            ixs.sort_by(|&a, &b| album_order(&tracks[a], &tracks[b]));
            let first = &tracks[ixs[0]];
            Album {
                title: first.album.clone().unwrap_or_default(),
                artist: first.album_artist_or_artist().map(str::to_string),
                year: ixs.iter().filter_map(|&i| tracks[i].year).min(),
                cover: ixs.iter().find_map(|&i| tracks[i].cover.clone()),
                duration: ixs.iter().map(|&i| tracks[i].duration).sum(),
                tracks: ixs,
            }
        })
        .collect()
}

/// Nom donné aux morceaux sans artiste.
pub const UNKNOWN_ARTIST: &str = "Artiste inconnu";

/// Artistes (artiste de l'album, à défaut du morceau), triés par nom ;
/// « Artiste inconnu » en dernier.
pub fn artists(tracks: &[Track]) -> Vec<Artist> {
    let mut groups: BTreeMap<(bool, String), (String, Vec<usize>)> = BTreeMap::new();
    for (ix, t) in tracks.iter().enumerate() {
        let known = t.album_artist_or_artist();
        let name = known.unwrap_or(UNKNOWN_ARTIST);
        groups
            .entry((known.is_none(), fold(name)))
            .or_insert_with(|| (name.to_string(), Vec::new()))
            .1
            .push(ix);
    }
    groups
        .into_values()
        .map(|(name, mut ixs)| {
            ixs.sort_by(|&a, &b| {
                let (ta, tb) = (&tracks[a], &tracks[b]);
                (ta.year, fold(ta.album.as_deref().unwrap_or_default()))
                    .cmp(&(tb.year, fold(tb.album.as_deref().unwrap_or_default())))
                    .then_with(|| album_order(ta, tb))
            });
            let mut albums: Vec<String> = ixs
                .iter()
                .filter_map(|&i| tracks[i].album.as_deref().map(fold))
                .collect();
            albums.dedup();
            Artist {
                name,
                albums: albums.len(),
                tracks: ixs,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(title: &str, artist: &str, album: Option<&str>, track: u32) -> Track {
        Track {
            title: title.into(),
            artist: Some(artist.into()),
            album: album.map(Into::into),
            track_no: Some(track),
            duration: Duration::from_secs(60),
            ..Default::default()
        }
    }

    #[test]
    fn groups_albums_and_artists() {
        let tracks = vec![
            t("B2", "Björk", Some("Post"), 2),
            t("A1", "Air", Some("Moon Safari"), 1),
            t("B1", "Bjork", Some("post"), 1), // même album malgré accents / casse
            t("Single", "Air", None, 1),
            Track {
                title: "Anonyme".into(),
                ..Default::default()
            },
        ];
        let albums = albums(&tracks);
        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].title, "Moon Safari");
        assert_eq!(albums[1].tracks, vec![2, 0]); // piste 1 puis 2
        assert_eq!(albums[1].duration, Duration::from_secs(120));

        let artists = artists(&tracks);
        assert_eq!(artists.len(), 3);
        assert_eq!(artists[2].name, UNKNOWN_ARTIST);
        assert_eq!(artists[0].name, "Air");
        assert_eq!(artists[0].tracks.len(), 2);
        assert_eq!(artists[0].albums, 1);
        assert_eq!(artists[1].tracks.len(), 2);
    }
}
