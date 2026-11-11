//! Infos sur les artistes, albums et titres.
//!
//! - fiches et discographies : [MusicBrainz](https://musicbrainz.org)
//!   (gratuit, sans clé, une requête par seconde) ;
//! - pochettes : [Cover Art Archive](https://coverartarchive.org) ;
//! - biographies et photos : Wikidata puis Wikipédia, en français si possible.
//!
//! Les biographies se demandent à part ([`Info::bio`]) pour ne pas retarder
//! les fiches. Les réponses sont gardées deux semaines en cache disque (un
//! jour pour une recherche sans résultat), puis supprimées au démarrage. Les fonctions
//! sont `async` et doivent tourner sur le runtime tokio (reqwest).

mod http;
mod musicbrainz;
mod wiki;

use std::path::PathBuf;

use musicbrainz::{
    API, ArtistSearch, MbArtist, MbRecording, MbRelease, MbReleaseGroup, RecordingSearch,
    ReleaseGroupBrowse, ReleaseGroupSearch, quoted, words,
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum InfoError {
    #[error("service injoignable : {0}")]
    Network(String),
    /// Le service déleste (503), même après plusieurs essais.
    #[error("service surchargé")]
    Busy,
    #[error("réponse inattendue : {0}")]
    Decode(String),
    #[error("introuvable")]
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistRef {
    /// Identifiant MusicBrainz.
    pub id: String,
    pub name: String,
}

/// Nature d'une parution, dans l'ordre d'affichage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlbumKind {
    Album,
    Ep,
    Single,
    /// Live, compilation, remix, bande originale…
    Other,
}

impl AlbumKind {
    pub fn rank(self) -> u8 {
        self as u8
    }

    pub fn label(self) -> &'static str {
        match self {
            AlbumKind::Album => "Album",
            AlbumKind::Ep => "EP",
            AlbumKind::Single => "Single",
            AlbumKind::Other => "Autre",
        }
    }
}

/// Une parution (« release group » de MusicBrainz) : toutes les éditions
/// d'un même album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseGroup {
    pub id: String,
    pub title: String,
    pub year: Option<u32>,
    pub kind: AlbumKind,
    /// « Live », « Compilation, Remix »…
    pub secondary: String,
    pub artist: String,
}

impl ReleaseGroup {
    /// Pochette (Cover Art Archive), peut ne pas exister.
    pub fn cover_url(&self, size: u32) -> String {
        cover_url(&self.id, size)
    }
}

pub fn cover_url(release_group: &str, size: u32) -> String {
    format!("https://coverartarchive.org/release-group/{release_group}/front-{size}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bio {
    pub extract: String,
    /// Article complet.
    pub url: String,
    pub image: Option<String>,
    /// `fr`, `en`.
    pub lang: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistInfo {
    pub id: String,
    pub name: String,
    /// « Groupe », « Personne »…
    pub kind: Option<String>,
    pub disambiguation: Option<String>,
    /// Ville ou région d'origine, pays.
    pub origin: Vec<String>,
    pub begin: Option<String>,
    pub end: Option<String>,
    pub genres: Vec<String>,
    /// Entité Wikidata, pour [`Info::bio`].
    pub wikidata: Option<String>,
    /// Albums et EP, triés par nature puis par date.
    pub discography: Vec<ReleaseGroup>,
    /// Nombre d'albums et EP sur MusicBrainz : plus que `discography` quand
    /// la liste est tronquée (artistes très prolifiques, classique).
    pub discography_total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumTrack {
    /// « 3 », ou « 2-3 » pour un album en plusieurs disques.
    pub position: String,
    pub title: String,
    pub length_ms: Option<u64>,
    pub recording_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumInfo {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artists: Vec<ArtistRef>,
    pub date: Option<String>,
    pub kind: AlbumKind,
    pub secondary: String,
    pub label: Option<String>,
    pub genres: Vec<String>,
    pub tracks: Vec<AlbumTrack>,
    /// Entité Wikidata, pour [`Info::bio`].
    pub wikidata: Option<String>,
}

impl AlbumInfo {
    pub fn cover_url(&self, size: u32) -> String {
        cover_url(&self.id, size)
    }
}

/// Personnes qui ont tenu un rôle sur un titre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credit {
    /// « Composition », « Chant »…
    pub role: String,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackInfo {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artists: Vec<ArtistRef>,
    pub length_ms: Option<u64>,
    pub first_release: Option<String>,
    pub genres: Vec<String>,
    pub credits: Vec<Credit>,
    /// Albums où paraît le titre, albums studio d'abord.
    pub albums: Vec<ReleaseGroup>,
}

/// Page MusicBrainz d'une entité (`artist`, `release-group`, `recording`).
pub fn musicbrainz_url(entity: &str, id: &str) -> String {
    format!("https://musicbrainz.org/{entity}/{id}")
}

/// Recherche dans MusicBrainz, sur son site.
pub fn musicbrainz_search_url(query: &str, kind: &str) -> String {
    let mut q = String::new();
    for b in query.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => q.push(b as char),
            b' ' => q.push('+'),
            _ => q.push_str(&format!("%{b:02X}")),
        }
    }
    format!("https://musicbrainz.org/search?query={q}&type={kind}")
}

pub struct Info {
    http: http::Http,
}

impl Info {
    /// `cache` : dossier des réponses et images (créé au besoin).
    pub fn new(cache: PathBuf) -> Result<Self, InfoError> {
        Ok(Self {
            http: http::Http::new(cache)?,
        })
    }

    /// Recherche, puis choix du meilleur résultat par `best`. Quand rien
    /// ne convient, la réponse ne reste qu'un jour en cache au lieu de
    /// deux semaines : la fiche peut être ajoutée ou corrigée d'ici là.
    async fn search<T: serde::de::DeserializeOwned>(
        &self,
        entity: &str,
        query: &str,
        limit: u32,
        best: impl FnOnce(&T) -> Option<String>,
    ) -> Result<String, InfoError> {
        let mut q = String::new();
        for b in query.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => q.push(b as char),
                _ => q.push_str(&format!("%{b:02X}")),
            }
        }
        let url = format!("{API}/{entity}?query={q}&limit={limit}&fmt=json");
        let found: T = self.http.json(&url).await?;
        best(&found).ok_or_else(|| {
            self.http.expire_soon(&url);
            InfoError::NotFound
        })
    }

    // ------------------------------------------------------------ artiste

    /// Identifiant de l'artiste le plus probable pour ce nom (ou cet alias).
    ///
    /// Pour une liste de musiciens (« A & B & C »), le premier est cherché
    /// si la liste entière ne donne rien.
    pub async fn find_artist(&self, name: &str) -> Result<String, InfoError> {
        let result = self.find_artist_named(name).await;
        match (result, first_artist(name)) {
            (Err(InfoError::NotFound), Some(first)) => self.find_artist_named(first).await,
            (result, _) => result,
        }
    }

    async fn find_artist_named(&self, name: &str) -> Result<String, InfoError> {
        self.search(
            "artist",
            &format!("{} OR {}", clause("artist", name), clause("alias", name)),
            10,
            |found: &ArtistSearch| musicbrainz::best_artist(&found.artists, name),
        )
        .await
    }

    pub async fn artist(&self, id: &str) -> Result<ArtistInfo, InfoError> {
        let a: MbArtist = self
            .http
            .json(&format!("{API}/artist/{id}?inc=url-rels+genres&fmt=json"))
            .await?;
        let groups: ReleaseGroupBrowse = self
            .http
            .json(&format!(
                "{API}/release-group?artist={id}&type=album|ep&limit=100&fmt=json"
            ))
            .await?;
        if groups.release_group_count > groups.release_groups.len() {
            tracing::debug!(
                id,
                total = groups.release_group_count,
                "discographie tronquée"
            );
        }
        let mut discography: Vec<ReleaseGroup> =
            groups.release_groups.iter().map(|g| g.summary()).collect();
        discography.sort_by_key(|g| (g.kind.rank(), g.year.unwrap_or(9999), g.title.clone()));
        let mut origin: Vec<String> = Vec::new();
        for place in [a.begin_area.as_ref(), a.area.as_ref()]
            .into_iter()
            .flatten()
        {
            if !place.name.is_empty() && !origin.contains(&place.name) {
                origin.push(place.name.clone());
            }
        }
        Ok(ArtistInfo {
            id: a.id,
            name: a.name,
            kind: a.kind.map(|k| match k.as_str() {
                "Group" => "Groupe".to_string(),
                "Person" => "Artiste".to_string(),
                "Orchestra" => "Orchestre".to_string(),
                "Choir" => "Chœur".to_string(),
                _ => k,
            }),
            disambiguation: Some(a.disambiguation).filter(|d| !d.is_empty()),
            origin,
            begin: a.life_span.begin,
            end: a.life_span.end,
            genres: musicbrainz::genres(&a.genres),
            wikidata: musicbrainz::wikidata_id(&a.relations),
            discography_total: groups.release_group_count.max(discography.len()),
            discography,
        })
    }

    // -------------------------------------------------------------- album

    pub async fn find_album(&self, artist: &str, title: &str) -> Result<String, InfoError> {
        self.search(
            "release-group",
            &format!(
                "{} AND {}",
                clause("releasegroup", title),
                clause("artist", artist)
            ),
            25,
            |found: &ReleaseGroupSearch| {
                musicbrainz::best_release_group(&found.release_groups, title, artist)
            },
        )
        .await
    }

    pub async fn album(&self, id: &str) -> Result<AlbumInfo, InfoError> {
        let g: MbReleaseGroup = self
            .http
            .json(&format!(
                "{API}/release-group/{id}?inc=artist-credits+releases+genres+url-rels&fmt=json"
            ))
            .await?;
        let release: Option<MbRelease> = match g.reference_release() {
            Some(r) => Some(
                self.http
                    .json(&format!(
                        "{API}/release/{}?inc=recordings+labels&fmt=json",
                        r.id
                    ))
                    .await?,
            ),
            None => None,
        };
        Ok(AlbumInfo {
            id: g.id.clone(),
            title: g.title.clone(),
            artist: musicbrainz::credit_text(&g.artist_credit),
            artists: musicbrainz::credit_artists(&g.artist_credit),
            date: Some(g.first_release_date.clone()).filter(|d| !d.is_empty()),
            kind: g.kind(),
            secondary: g.secondary_types.join(", "),
            label: release.as_ref().and_then(MbRelease::label),
            genres: musicbrainz::genres(&g.genres),
            tracks: release.as_ref().map(MbRelease::tracks).unwrap_or_default(),
            wikidata: musicbrainz::wikidata_id(&g.relations),
        })
    }

    // ---------------------------------------------------------- biographie

    /// Résumé Wikipédia de l'entité Wikidata d'une fiche, s'il y en a un.
    /// À part des fiches : elles s'affichent sans l'attendre.
    pub async fn bio(&self, wikidata: &str) -> Option<Bio> {
        wiki::bio(&self.http, wikidata).await
    }

    // -------------------------------------------------------------- titre

    /// Enregistrement le plus probable, de préférence celui de l'album.
    ///
    /// Une seule recherche : le titre exact est favorisé, mais ses mots
    /// suffisent — les titres des radios diffèrent souvent de MusicBrainz
    /// par une apostrophe, un tiret ou une précision (« [ft. …] »,
    /// « (club mix) »). Les radios créditent aussi souvent tous les
    /// musiciens (« A & B & C ») là où MusicBrainz ne nomme que le premier :
    /// celui-ci est cherché seul si la liste entière ne donne rien.
    pub async fn find_track(
        &self,
        artist: &str,
        title: &str,
        album: Option<&str>,
    ) -> Result<String, InfoError> {
        let result = self.find_track_by(artist, title, album).await;
        match (result, first_artist(artist)) {
            (Err(InfoError::NotFound), Some(first)) => {
                self.find_track_by(first, title, album).await
            }
            (result, _) => result,
        }
    }

    async fn find_track_by(
        &self,
        artist: &str,
        title: &str,
        album: Option<&str>,
    ) -> Result<String, InfoError> {
        let album = album.map(str::trim).filter(|a| !a.is_empty());
        let mut query = format!(
            "+{} +{}",
            clause("recording", title),
            clause("artist", artist)
        );
        if let Some(album) = album {
            // Facultatif : fait remonter les enregistrements de l'album.
            query.push(' ');
            query.push_str(&clause("release", album));
        }
        self.search("recording", &query, 25, |found: &RecordingSearch| {
            musicbrainz::best_recording(&found.recordings, title, artist, album)
        })
        .await
    }

    pub async fn track(&self, id: &str) -> Result<TrackInfo, InfoError> {
        let r: MbRecording = self
            .http
            .json(&format!(
                "{API}/recording/{id}?inc=artist-credits+releases+release-groups+genres\
                 +artist-rels+work-rels+work-level-rels&fmt=json"
            ))
            .await?;
        Ok(TrackInfo {
            id: r.id.clone(),
            title: r.title.clone(),
            artist: musicbrainz::credit_text(&r.artist_credit),
            artists: musicbrainz::credit_artists(&r.artist_credit),
            length_ms: r.length,
            first_release: Some(r.first_release_date.clone()).filter(|d| !d.is_empty()),
            genres: musicbrainz::genres(&r.genres),
            credits: r.credits(),
            albums: r.albums(),
        })
    }

    // ------------------------------------------------------------- images

    /// Image téléchargée dans le cache ; `None` si elle n'existe pas.
    pub async fn image(&self, url: &str) -> Result<Option<PathBuf>, InfoError> {
        self.http.image(url).await
    }
}

/// Premier nom d'une liste de musiciens (« A & B », « A, B », « A feat.
/// B », « A / B ») ; `None` s'il n'y en a qu'un.
fn first_artist(names: &str) -> Option<&str> {
    // Minuscules ASCII seulement : les positions restent celles de `names`.
    let lower = names.to_ascii_lowercase();
    [
        " & ",
        ", ",
        " / ",
        " feat. ",
        " feat ",
        " ft. ",
        " featuring ",
    ]
    .iter()
    // Les séparateurs sont en ASCII : même position dans `lower`.
    .filter_map(|sep| lower.find(sep))
    .min()
    .map(|at| names[..at].trim())
    .filter(|first| !first.is_empty())
}

/// Critère de recherche sur un champ : la phrase exacte, favorisée, ou
/// ses mots (sans ponctuation ni précisions) ; la phrase seule s'il ne
/// reste aucun mot (« !!! »).
fn clause(field: &str, text: &str) -> String {
    let (exact, words) = (quoted(text.trim()), words(text));
    if words.is_empty() {
        format!("{field}:({exact})")
    } else {
        format!("{field}:({exact}^4 OR ({words}))")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn clauses() {
        assert_eq!(
            super::clause("recording", "Sweet Rub-A-Dub [ft. X]"),
            r#"recording:("Sweet Rub-A-Dub [ft. X]"^4 OR (sweet rub a dub))"#
        );
        assert_eq!(super::clause("artist", "!!!"), r#"artist:("!!!")"#);
    }

    #[test]
    fn first_artists() {
        use super::first_artist;
        assert_eq!(
            first_artist("Herbie Mann & Cissy Houston & Pat Rebillot"),
            Some("Herbie Mann")
        );
        assert_eq!(
            first_artist("Isabella Lundgren, Carl Bagge"),
            Some("Isabella Lundgren")
        );
        assert_eq!(
            first_artist("Black Roots Feat. Dub Judah"),
            Some("Black Roots")
        );
        assert_eq!(first_artist("Massive Attack"), None);
        assert_eq!(first_artist("Ébène & Co"), Some("Ébène"));
        assert_eq!(first_artist(" & X"), None);
        assert_eq!(first_artist("İlhan & B"), Some("İlhan"));
    }

    #[test]
    fn search_url() {
        assert_eq!(
            super::musicbrainz_search_url("Björk Homogenic", "release_group"),
            "https://musicbrainz.org/search?query=Bj%C3%B6rk+Homogenic&type=release_group"
        );
    }
}
