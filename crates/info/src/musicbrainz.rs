//! Réponses de l'API MusicBrainz (<https://musicbrainz.org/doc/MusicBrainz_API>)
//! et choix des meilleurs résultats de recherche.

use std::cmp::Reverse;

use ondes_library::search::fold;
use serde::Deserialize;

use crate::{AlbumKind, AlbumTrack, ArtistRef, Credit, ReleaseGroup};

pub const API: &str = "https://musicbrainz.org/ws/2";

// ---------------------------------------------------------------- JSON

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ArtistCredit {
    pub name: String,
    pub joinphrase: String,
    pub artist: Option<MbArtistRef>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct MbArtistRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Genre {
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct UrlRelation {
    #[serde(rename = "type")]
    pub kind: String,
    pub url: Option<UrlTarget>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct UrlTarget {
    pub resource: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct MbReleaseGroup {
    pub id: String,
    pub title: String,
    pub first_release_date: String,
    pub primary_type: Option<String>,
    pub secondary_types: Vec<String>,
    pub artist_credit: Vec<ArtistCredit>,
    pub releases: Vec<MbRelease>,
    pub genres: Vec<Genre>,
    pub relations: Vec<UrlRelation>,
    pub score: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct MbRelease {
    pub id: String,
    pub title: String,
    pub date: String,
    pub status: Option<String>,
    pub release_group: Option<MbReleaseGroup>,
    pub media: Vec<Medium>,
    pub label_info: Vec<LabelInfo>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Medium {
    pub position: u32,
    pub tracks: Vec<MbTrack>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct MbTrack {
    pub number: String,
    pub title: String,
    pub length: Option<u64>,
    pub recording: Option<MbRecordingRef>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct MbRecordingRef {
    pub id: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LabelInfo {
    pub label: Option<Named>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Named {
    pub name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct MbArtist {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub disambiguation: String,
    pub country: Option<String>,
    pub area: Option<Named>,
    pub begin_area: Option<Named>,
    pub life_span: LifeSpan,
    pub genres: Vec<Genre>,
    pub relations: Vec<UrlRelation>,
    /// Autres noms (dans les résultats de recherche).
    pub aliases: Vec<Named>,
    pub score: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LifeSpan {
    pub begin: Option<String>,
    pub end: Option<String>,
    pub ended: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct MbRecording {
    pub id: String,
    pub title: String,
    pub length: Option<u64>,
    pub first_release_date: String,
    pub artist_credit: Vec<ArtistCredit>,
    pub releases: Vec<MbRelease>,
    pub genres: Vec<Genre>,
    pub relations: Vec<Relation>,
    pub score: i64,
}

/// Relation d'un enregistrement : avec un artiste, ou avec l'œuvre jouée
/// (qui porte compositeurs et paroliers).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Relation {
    #[serde(rename = "type")]
    pub kind: String,
    pub attributes: Vec<String>,
    pub artist: Option<MbArtistRef>,
    pub work: Option<Work>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Work {
    pub relations: Vec<Relation>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ArtistSearch {
    pub artists: Vec<MbArtist>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ReleaseGroupSearch {
    pub release_groups: Vec<MbReleaseGroup>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ReleaseGroupBrowse {
    pub release_groups: Vec<MbReleaseGroup>,
    pub release_group_count: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RecordingSearch {
    pub recordings: Vec<MbRecording>,
}

// ---------------------------------------------------------- requêtes

/// Valeur entre guillemets dans une requête Lucene.
pub fn quoted(value: &str) -> String {
    let escaped: String = value
        .chars()
        .flat_map(|c| match c {
            '"' | '\\' => vec!['\\', c],
            c => vec![c],
        })
        .collect();
    format!("\"{escaped}\"")
}

// -------------------------------------------------------- conversions

pub fn credit_text(credits: &[ArtistCredit]) -> String {
    credits
        .iter()
        .map(|c| format!("{}{}", c.name, c.joinphrase))
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn credit_artists(credits: &[ArtistCredit]) -> Vec<ArtistRef> {
    credits
        .iter()
        .filter_map(|c| c.artist.as_ref())
        .map(|a| ArtistRef {
            id: a.id.clone(),
            name: a.name.clone(),
        })
        .collect()
}

/// Genres les plus votés d'abord.
pub fn genres(list: &[Genre]) -> Vec<String> {
    let mut list: Vec<&Genre> = list.iter().collect();
    list.sort_by_key(|g| Reverse(g.count));
    list.iter().take(6).map(|g| g.name.clone()).collect()
}

pub fn wikidata_id(relations: &[UrlRelation]) -> Option<String> {
    relations
        .iter()
        .filter(|r| r.kind == "wikidata")
        .find_map(|r| {
            r.url
                .as_ref()?
                .resource
                .rsplit('/')
                .next()
                .map(String::from)
        })
        .filter(|q| q.starts_with('Q'))
}

pub fn year(date: &str) -> Option<u32> {
    date.get(..4)?.parse().ok()
}

impl MbReleaseGroup {
    pub fn kind(&self) -> AlbumKind {
        match self.primary_type.as_deref() {
            _ if !self.secondary_types.is_empty() => AlbumKind::Other,
            Some("Album") => AlbumKind::Album,
            Some("EP") => AlbumKind::Ep,
            Some("Single") => AlbumKind::Single,
            _ => AlbumKind::Other,
        }
    }

    pub fn summary(&self) -> ReleaseGroup {
        ReleaseGroup {
            id: self.id.clone(),
            title: self.title.clone(),
            year: year(&self.first_release_date),
            kind: self.kind(),
            secondary: self.secondary_types.join(", "),
            artist: credit_text(&self.artist_credit),
        }
    }

    /// Sortie de référence : la première officielle (sinon la première).
    pub fn reference_release(&self) -> Option<&MbRelease> {
        let dated = |r: &&MbRelease| {
            if r.date.is_empty() {
                "9999".into()
            } else {
                r.date.clone()
            }
        };
        self.releases
            .iter()
            .filter(|r| r.status.as_deref() == Some("Official"))
            .min_by_key(dated)
            .or_else(|| self.releases.iter().min_by_key(dated))
    }
}

impl MbRelease {
    pub fn tracks(&self) -> Vec<AlbumTrack> {
        let discs = self.media.len();
        self.media
            .iter()
            .flat_map(|m| {
                m.tracks.iter().map(move |t| AlbumTrack {
                    position: if discs > 1 {
                        format!("{}-{}", m.position, t.number)
                    } else {
                        t.number.clone()
                    },
                    title: t.title.clone(),
                    length_ms: t.length,
                    recording_id: t.recording.as_ref().map(|r| r.id.clone()),
                })
            })
            .collect()
    }

    pub fn label(&self) -> Option<String> {
        self.label_info
            .iter()
            .find_map(|l| l.label.as_ref().map(|l| l.name.clone()))
            .filter(|l| !l.is_empty() && l != "[no label]")
    }
}

impl MbRecording {
    /// Albums où paraît l'enregistrement : albums studio d'abord, puis par date.
    pub fn albums(&self) -> Vec<ReleaseGroup> {
        let mut seen = std::collections::HashSet::new();
        let mut list: Vec<ReleaseGroup> = self
            .releases
            .iter()
            .filter_map(|r| r.release_group.as_ref())
            .filter(|g| seen.insert(g.id.clone()))
            .map(|g| {
                let mut summary = g.summary();
                if summary.artist.is_empty() {
                    summary.artist = credit_text(&self.artist_credit);
                }
                summary
            })
            .collect();
        list.sort_by_key(|g| (g.kind.rank(), g.year.unwrap_or(9999)));
        list
    }

    /// Crédits regroupés par rôle, dans un ordre lisible ; une personne
    /// n'apparaît qu'une fois par rôle, avec toutes ses précisions.
    pub fn credits(&self) -> Vec<Credit> {
        // (rôle, nom, précisions : instrument, « chant principal »…)
        let mut found: Vec<(&'static str, String, Vec<String>)> = Vec::new();
        let mut push = |rel: &Relation| {
            let (Some(role), Some(artist)) = (role(&rel.kind), &rel.artist) else {
                return;
            };
            let ix = match found
                .iter()
                .position(|(r, n, _)| *r == role && *n == artist.name)
            {
                Some(ix) => ix,
                None => {
                    found.push((role, artist.name.clone(), Vec::new()));
                    found.len() - 1
                }
            };
            for attr in &rel.attributes {
                let attr = attribute(attr);
                if !found[ix].2.contains(&attr) {
                    found[ix].2.push(attr);
                }
            }
        };
        for rel in &self.relations {
            push(rel);
            if let Some(work) = &rel.work {
                work.relations.iter().for_each(&mut push);
            }
        }
        ROLES
            .iter()
            .filter_map(|&(_, label)| {
                let names: Vec<String> = found
                    .iter()
                    .filter(|(r, _, _)| *r == label)
                    .map(|(_, name, attrs)| {
                        if attrs.is_empty() {
                            name.clone()
                        } else {
                            format!("{name} ({})", attrs.join(", "))
                        }
                    })
                    .collect();
                (!names.is_empty()).then(|| Credit {
                    role: label.to_string(),
                    names,
                })
            })
            .collect()
    }

    /// Paraît sur un album studio officiel.
    fn on_studio_album(&self) -> bool {
        self.releases.iter().any(|r| {
            r.status.as_deref() == Some("Official")
                && r.release_group
                    .as_ref()
                    .is_some_and(|g| g.kind() == AlbumKind::Album)
        })
    }
}

/// Rôles affichés (type MusicBrainz → libellé), dans l'ordre d'affichage.
const ROLES: &[(&str, &str)] = &[
    ("composer", "Composition"),
    ("lyricist", "Paroles"),
    ("writer", "Écriture"),
    ("librettist", "Livret"),
    ("vocal", "Chant"),
    ("instrument", "Instruments"),
    ("performer", "Interprétation"),
    ("conductor", "Direction"),
    ("performing orchestra", "Orchestre"),
    ("arranger", "Arrangement"),
    ("producer", "Production"),
    ("mix", "Mixage"),
    ("recording", "Enregistrement"),
    ("engineer", "Ingénierie du son"),
];

/// Précisions courantes des crédits, en français.
const ATTRIBUTES: &[(&str, &str)] = &[
    ("lead vocals", "chant principal"),
    ("background vocals", "chœurs"),
    ("guest", "invité·e"),
    ("additional", "additionnel"),
    ("guitar", "guitare"),
    ("electric guitar", "guitare électrique"),
    ("acoustic guitar", "guitare acoustique"),
    ("bass guitar", "basse"),
    ("electric bass guitar", "basse électrique"),
    ("bass", "basse"),
    ("double bass", "contrebasse"),
    ("drums (drum set)", "batterie"),
    ("drums", "batterie"),
    ("percussion", "percussions"),
    ("keyboard", "claviers"),
    ("synthesizer", "synthétiseur"),
    ("organ", "orgue"),
    ("strings", "cordes"),
    ("violin", "violon"),
    ("viola", "alto"),
    ("cello", "violoncelle"),
    ("trumpet", "trompette"),
    ("trombone", "trombone"),
    ("flute", "flûte"),
    ("clarinet", "clarinette"),
    ("horn", "cor"),
    ("programming", "programmation"),
    ("turntables", "platines"),
];

fn attribute(attr: &str) -> String {
    ATTRIBUTES
        .iter()
        .find(|(en, _)| en.eq_ignore_ascii_case(attr))
        .map_or_else(|| attr.to_string(), |&(_, fr)| fr.to_string())
}

fn role(kind: &str) -> Option<&'static str> {
    ROLES.iter().find(|(k, _)| *k == kind).map(|&(_, l)| l)
}

// ------------------------------------------------------------- choix

/// Titre comparable : sans accents ni casse, ni précisions entre
/// parenthèses ou crochets (« (Remastered) », « [Live] »).
fn base_title(title: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in fold(title).chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Mots comparables, en minuscules séparés d'une espace : sans accents,
/// ponctuation, précisions entre parenthèses ni invités (« feat. X »).
/// Sert aussi de requête souple (aucun opérateur Lucene n'y survit).
pub fn words(text: &str) -> String {
    let mut base = base_title(text);
    for mark in [" feat. ", " feat ", " ft. ", " featuring "] {
        if let Some(at) = base.find(mark) {
            base.truncate(at);
        }
    }
    base.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 2 : titre identique ; 1 : mêmes mots (sans parenthèses ni ponctuation) ;
/// 0 : autre.
fn title_match(found: &str, wanted: &str) -> u8 {
    if fold(found).trim() == fold(wanted).trim() {
        return 2;
    }
    let wanted = words(wanted);
    u8::from(!wanted.is_empty() && words(found) == wanted)
}

/// Place de `wanted` dans le crédit : 2 s'il est l'un des artistes
/// crédités (ou tout le crédit), 1 si son nom n'est que contenu dans un
/// autre (« Air » dans « Air Supply »), 0 s'il n'y est pas.
fn credited(credits: &[ArtistCredit], wanted: &str) -> u8 {
    let wanted = words(wanted);
    if wanted.is_empty() {
        return 0;
    }
    let names = std::iter::once(credit_text(credits)).chain(credits.iter().flat_map(|c| {
        std::iter::once(c.name.clone()).chain(c.artist.as_ref().map(|a| a.name.clone()))
    }));
    let padded = format!(" {wanted} ");
    names
        .map(|name| {
            let name = words(&name);
            if name == wanted {
                2
            } else {
                u8::from(format!(" {name} ").contains(&padded))
            }
        })
        .max()
        .unwrap_or(0)
}

/// Meilleur artiste : il doit porter ce nom ou l'avoir pour alias (la
/// recherche ramène aussi ceux qui n'en partagent qu'un mot) ; nom
/// identique d'abord, puis le score de MusicBrainz.
pub fn best_artist(found: &[MbArtist], name: &str) -> Option<String> {
    let rank = |a: &MbArtist| {
        let alias = a.aliases.iter().map(|n| title_match(&n.name, name)).max();
        title_match(&a.name, name).max(alias.unwrap_or(0))
    };
    found
        .iter()
        .filter(|a| rank(a) > 0)
        .max_by_key(|a| (rank(a), a.score))
        .map(|a| a.id.clone())
}

/// Place du bon artiste (voir [`credited`]) : la recherche ramène aussi
/// ceux qui n'en partagent qu'un mot. Sans mot à comparer (« !!! »), le
/// score de MusicBrainz décide.
fn artist_rank(score: i64, credits: &[ArtistCredit], artist: &str) -> u8 {
    if words(artist).is_empty() {
        if score >= 70 { 2 } else { 0 }
    } else {
        credited(credits, artist)
    }
}

/// Meilleur album de l'artiste : artiste exact, titre identique, album
/// studio, score.
pub fn best_release_group(found: &[MbReleaseGroup], title: &str, artist: &str) -> Option<String> {
    found
        .iter()
        .map(|g| (g, artist_rank(g.score, &g.artist_credit, artist)))
        .filter(|(g, rank)| *rank > 0 && title_match(&g.title, title) > 0)
        .max_by_key(|(g, rank)| {
            (
                *rank,
                title_match(&g.title, title),
                Reverse(g.kind().rank()),
                g.score,
            )
        })
        .map(|(g, _)| g.id.clone())
}

/// Meilleur enregistrement de l'artiste : artiste exact, titre identique,
/// paru sur `album` s'il est donné, puis sur un album studio officiel, le
/// plus ancien. Les remix et versions live passent après.
pub fn best_recording(
    found: &[MbRecording],
    title: &str,
    artist: &str,
    album: Option<&str>,
) -> Option<String> {
    let on_album = |r: &MbRecording| {
        album.is_some_and(|album| r.releases.iter().any(|x| title_match(&x.title, album) > 0))
    };
    found
        .iter()
        .map(|r| (r, artist_rank(r.score, &r.artist_credit, artist)))
        .filter(|(r, rank)| *rank > 0 && title_match(&r.title, title) > 0)
        .min_by_key(|(r, rank)| {
            (
                Reverse(*rank),
                Reverse(title_match(&r.title, title)),
                !on_album(r),
                !r.on_studio_album(),
                if r.first_release_date.is_empty() {
                    "9999".to_string()
                } else {
                    r.first_release_date.clone()
                },
            )
        })
        .map(|(r, _)| r.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(json: &str) -> MbRecording {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn quoting() {
        assert_eq!(quoted(r#"Say "hi" \o/"#), r#""Say \"hi\" \\o/""#);
    }

    #[test]
    fn title_matching() {
        assert_eq!(title_match("Déjà Vu", "deja vu"), 2);
        assert_eq!(title_match("Teardrop (Remastered 2019)", "Teardrop"), 1);
        assert_eq!(title_match("Teardrop", "Angel"), 0);
        // Écarts courants entre les titres des radios et MusicBrainz.
        assert_eq!(title_match("Love’s the Reason", "Love`s The Reason"), 1);
        assert_eq!(title_match("Sweet Rub a Dub", "Sweet Rub-A-Dub"), 1);
        assert_eq!(title_match("Singin’ in the Rain", "Singin in the Rain"), 1);
        assert_eq!(
            title_match("What Them a Do", "What Them A Do [ft. Dub Judah]"),
            1
        );
        assert_eq!(title_match("Melody", "Melody (1983 club vinyl mix)"), 1);
        assert_eq!(title_match("(Intro)", "(Outro)"), 0);
    }

    const MA: &str = r#""artist-credit":[{"name":"Massive Attack","artist":{"id":"ma","name":"Massive Attack"}}]"#;

    #[test]
    fn prefers_original_studio_recording() {
        let found = vec![
            rec(&format!(
                r#"{{"id":"live","title":"Teardrop","score":100,"first-release-date":"1998",{MA},
                "releases":[{{"title":"Brussels","status":"Bootleg","release-group":{{"id":"g1","primary-type":"Album","secondary-types":["Live"]}}}}]}}"#
            )),
            rec(&format!(
                r#"{{"id":"remix","title":"Teardrop (Mazaruni dub)","score":100,"first-release-date":"2019",{MA},
                "releases":[{{"status":"Official","release-group":{{"id":"g2","primary-type":"Album"}}}}]}}"#
            )),
            rec(&format!(
                r#"{{"id":"single","title":"Teardrop","score":100,"first-release-date":"1998-04-27",{MA},
                "releases":[{{"title":"Mezzanine","status":"Official","release-group":{{"id":"g3","primary-type":"Album"}}}}]}}"#
            )),
            rec(&format!(
                r#"{{"id":"later","title":"Teardrop","score":100,"first-release-date":"2006",{MA},
                "releases":[{{"title":"Collected","status":"Official","release-group":{{"id":"g4","primary-type":"Album"}}}}]}}"#
            )),
        ];
        let find = |title, album| best_recording(&found, title, "Massive Attack", album);
        assert_eq!(find("Teardrop", None).as_deref(), Some("single"));
        assert_eq!(find("Inconnu", None), None);
        // L'album demandé passe avant l'ancienneté.
        assert_eq!(
            find("Teardrop", Some("collected")).as_deref(),
            Some("later")
        );
        assert_eq!(
            find("Teardrop", Some("Ailleurs")).as_deref(),
            Some("single")
        );
    }

    #[test]
    fn needs_the_artist() {
        let found = vec![
            rec(r#"{"id":"other","title":"Singin' In The Rain","score":100,
                "artist-credit":[{"name":"Jan Lundgren","artist":{"id":"a","name":"Jan Lundgren"}}]}"#),
            rec(r#"{"id":"right","title":"Singin’ in the Rain","score":40,
                "artist-credit":[{"name":"Isabella Lundgren","joinphrase":", ","artist":{"id":"b","name":"Isabella Lundgren"}},
                                 {"name":"Carl Bagge","artist":{"id":"c","name":"Carl Bagge"}}]}"#),
        ];
        let find = |artist| best_recording(&found, "Singin in the Rain", artist, None);
        assert_eq!(find("Isabella Lundgren").as_deref(), Some("right"));
        assert_eq!(
            find("Isabella Lundgren feat. Someone").as_deref(),
            Some("right")
        );
        assert_eq!(
            find("Isabella Lundgren, Carl Bagge").as_deref(),
            Some("right")
        );
        assert_eq!(find("Isabella").as_deref(), Some("right"));
        assert_eq!(find("Bella Lundgren"), None);
        // Sans mot à comparer, le score de MusicBrainz décide.
        assert_eq!(find("!!!").as_deref(), Some("other"));
    }

    #[test]
    fn exact_artist_first() {
        let found = vec![
            rec(r#"{"id":"supply","title":"Playground Love","score":100,
                "artist-credit":[{"name":"Air Supply","artist":{"id":"s","name":"Air Supply"}}]}"#),
            rec(
                r#"{"id":"air","title":"Playground Love (Remastered)","score":90,
                "artist-credit":[{"name":"Air","artist":{"id":"a","name":"Air"}}]}"#,
            ),
        ];
        assert_eq!(
            best_recording(&found, "Playground Love", "Air", None).as_deref(),
            Some("air")
        );
        assert_eq!(
            best_recording(&found, "Playground Love", "Air Supply", None).as_deref(),
            Some("supply")
        );
    }

    #[test]
    fn artist_by_name_or_alias() {
        let found: Vec<MbArtist> = serde_json::from_str(
            r#"[{"id":"brant","name":"Brant Bjork","score":75},
                {"id":"bjork","name":"Björk","score":100,"aliases":[{"name":"Bjork Gudmundsdottir"}]},
                {"id":"acdc","name":"AC/DC","score":60}]"#,
        )
        .unwrap();
        assert_eq!(best_artist(&found, "Bjork").as_deref(), Some("bjork"));
        assert_eq!(
            best_artist(&found, "bjork gudmundsdottir").as_deref(),
            Some("bjork")
        );
        assert_eq!(best_artist(&found, "AC DC").as_deref(), Some("acdc"));
        assert_eq!(best_artist(&found, "Bjorn"), None);
    }

    #[test]
    fn credits_from_recording_and_work() {
        let r = rec(r#"{"relations":[
            {"type":"vocal","attributes":["guest"],"artist":{"id":"a","name":"Elizabeth Fraser"}},
            {"type":"vocal","attributes":["guest","lead vocals"],"artist":{"id":"a","name":"Elizabeth Fraser"}},
            {"type":"instrument","attributes":["guitar"],"artist":{"id":"b","name":"Angelo Bruschini"}},
            {"type":"performance","work":{"relations":[
                {"type":"writer","artist":{"id":"c","name":"Robert Del Naja"}},
                {"type":"composer","artist":{"id":"d","name":"Andrew Vowles"}},
                {"type":"revision of"}
            ]}}
        ]}"#);
        let credits = r.credits();
        let roles: Vec<&str> = credits.iter().map(|c| c.role.as_str()).collect();
        assert_eq!(roles, ["Composition", "Écriture", "Chant", "Instruments"]);
        assert_eq!(
            credits[2].names,
            ["Elizabeth Fraser (invité·e, chant principal)"]
        );
    }

    #[test]
    fn album_tracks_and_reference_release() {
        let g: MbReleaseGroup = serde_json::from_str(
            r#"{"id":"g","title":"Mezzanine","first-release-date":"1998-04-20","primary-type":"Album",
            "releases":[{"id":"bootleg","date":"1997","status":"Bootleg"},
                        {"id":"reissue","date":"2018","status":"Official"},
                        {"id":"first","date":"1998-04-20","status":"Official"}]}"#,
        )
        .unwrap();
        assert_eq!(g.reference_release().unwrap().id, "first");
        assert_eq!(g.summary().year, Some(1998));
        assert_eq!(g.kind(), AlbumKind::Album);

        let r: MbRelease = serde_json::from_str(
            r#"{"media":[{"position":1,"tracks":[{"number":"1","title":"Angel","length":379000}]},
                         {"position":2,"tracks":[{"number":"1","title":"Bonus"}]}],
               "label-info":[{"label":{"name":"Circa"}}]}"#,
        )
        .unwrap();
        let tracks = r.tracks();
        assert_eq!(tracks[0].position, "1-1");
        assert_eq!(tracks[1].title, "Bonus");
        assert_eq!(r.label().as_deref(), Some("Circa"));
    }
}
