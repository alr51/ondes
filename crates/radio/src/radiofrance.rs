//! Titres en cours des stations Radio France.
//!
//! Leurs flux (Icecast ou HLS) n'envoient pas de métadonnées ICY : le titre
//! vient de l'API publique `livemeta` (celle des lecteurs du site), interrogée
//! par identifiant de station. Les identifiants ont été relevés sur l'API
//! elle-même (`/livemeta/live/{id}/transistor_musical_player` donne le nom).

use std::time::Duration;

use serde::Deserialize;

use crate::RadioError;

const API: &str = "https://api.radiofrance.fr/livemeta/pull";

/// Nom du flux (sans qualité ni extension) → identifiant `livemeta`.
const STATIONS: &[(&str, u32)] = &[
    ("franceinter", 1),
    ("franceinfo", 2),
    ("francemusique", 4),
    ("franceculture", 5),
    ("mouv", 6),
    // FIP et ses webradios (anciens noms `fip-webradioN` compris).
    ("fip", 7),
    ("fiprock", 64),
    ("fip-webradio4", 64),
    ("fipjazz", 65),
    ("fipgroove", 66),
    ("fip-webradio3", 66),
    ("fipworld", 69),
    ("fipnouveautes", 70),
    ("fip-webradio5", 70),
    ("fipreggae", 71),
    ("fip-webradio6", 71),
    ("fipelectro", 74),
    ("fip-webradio8", 74),
    ("fipmetal", 77),
    ("fippop", 78),
    ("fiphiphop", 95),
    ("fipsacrefrancais", 96),
    // Webradios de France Musique.
    ("francemusiqueeasyclassique", 401),
    ("francemusiqueclassiqueplus", 402),
    ("francemusiqueconcertsradiofrance", 403),
    ("francemusiqueocoramonde", 404),
    ("francemusiquelajazz", 405),
    ("francemusiquelacontemporaine", 406),
    ("francemusiquelabo", 407),
    ("francemusiquebaroque", 408),
    ("francemusiqueopera", 409),
    // ICI (ex-France Bleu).
    ("fbfrequenzamora", 11),
    ("fbalsace", 12),
    ("fbarmorique", 13),
    ("fbauxerre", 14),
    ("fbbearn", 15),
    ("fbbelfort", 16),
    ("fbberry", 17),
    ("fbbesancon", 18),
    ("fbbourgogne", 19),
    ("fbbreizizel", 20),
    ("fbchampagne", 21),
    ("fbcotentin", 22),
    ("fbcreuse", 23),
    ("fbdromeardeche", 24),
    ("fbgardlozere", 25),
    ("fbgascogne", 26),
    ("fbgironde", 27),
    ("fbherault", 28),
    ("fbisere", 29),
    ("fblarochelle", 30),
    ("fblimousin", 31),
    ("fbloireocean", 32),
    ("fbsudlorraine", 33),
    ("fbmayenne", 34),
    ("fbnord", 36),
    ("fbbassenormandie", 37),
    ("fbhautenormandie", 38),
    ("fborleans", 39),
    ("fbpaysdauvergne", 40),
    ("fbpaysbasque", 41),
    ("fbpaysdesavoie", 42),
    ("fbperigord", 43),
    ("fbpicardie", 44),
    ("fbprovence", 45),
    ("fbroussillon", 46),
    ("fbtouraine", 47),
    ("fbvaucluse", 48),
    ("fbazur", 49),
    ("fblorrainenord", 50),
    ("fbpoitou", 54),
    ("fb1071", 68),
    ("fbelsass", 90),
    ("fbmaine", 91),
    ("fbtoulouse", 92),
    ("fbstetienne", 93),
];

/// Domaines des flux Radio France (actuels et anciens).
const HOSTS: &[&str] = &[
    "radiofrance.fr",
    "fipradio.fr",
    "franceinter.fr",
    "franceinfo.fr",
    "francemusique.fr",
    "franceculture.fr",
    "francebleu.fr",
    "mouv.fr",
    "dvmr.fr",
];

/// Identifiant `livemeta` de la station d'une URL de flux Radio France.
///
/// `https://icecast.radiofrance.fr/fiprock-hifi.aac` → 64,
/// `https://stream.radiofrance.fr/fip/fip_hifi.m3u8?id=radiofrance` → 7.
pub fn station_id(url: &str) -> Option<u32> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.split(['?', '#']).next()?;
    let (host, path) = rest.split_once('/')?;
    let host = host.split(':').next()?.to_ascii_lowercase();
    if !HOSTS
        .iter()
        .any(|h| host == *h || host.ends_with(&format!(".{h}")))
    {
        return None;
    }
    let file = path.rsplit('/').next()?.to_ascii_lowercase();
    let mut name = file.split('.').next()?;
    for suffix in ["-lofi", "-midfi", "-hifi", "_lofi", "_midfi", "_hifi"] {
        name = name.strip_suffix(suffix).unwrap_or(name);
    }
    STATIONS
        .iter()
        .find(|(slug, _)| *slug == name)
        .map(|&(_, id)| id)
}

/// Ce qui passe à l'antenne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnAir {
    /// « Artiste - Titre » pour un morceau, sinon titre de l'émission.
    pub text: String,
    /// Fin prévue (secondes Unix).
    pub end: Option<i64>,
    /// Pour une émission : où en sont les morceaux qu'elle contient.
    pub songs: Songs,
}

/// Morceaux d'une station, quand c'est l'émission qui est à l'antenne.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Songs {
    /// Pas de morceaux annoncés (ou c'est un morceau qui passe).
    None,
    /// Le dernier morceau connu a fini à cet instant : le suivant n'est
    /// peut-être pas encore publié.
    Ended(i64),
    /// Le prochain morceau, déjà publié, commence à cet instant.
    Next(i64),
}

#[derive(Debug, Deserialize)]
struct Pull {
    #[serde(default)]
    steps: std::collections::HashMap<String, Step>,
    #[serde(default)]
    levels: Vec<Level>,
}

#[derive(Debug, Deserialize)]
struct Level {
    #[serde(default)]
    items: Vec<String>,
    #[serde(default)]
    position: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    title_concept: Option<String>,
    start: i64,
    end: i64,
    #[serde(default)]
    embed_type: Option<String>,
    /// Chaîne pour un morceau, liste d'identifiants pour une émission.
    #[serde(default)]
    authors: serde_json::Value,
    #[serde(default)]
    composers: Option<String>,
    #[serde(default)]
    performers: Option<String>,
}

impl Step {
    fn text(&self) -> Option<String> {
        let title = clean(self.title.as_deref())?;
        if self.embed_type.as_deref() == Some("song") {
            // Interprète en variété, compositeur en classique.
            let artist = [
                self.authors.as_str(),
                self.composers.as_deref(),
                self.performers.as_deref(),
            ]
            .into_iter()
            .find_map(clean)
            .map(|a| dedup_names(&a));
            return Some(match artist {
                Some(artist) => format!("{artist} - {title}"),
                None => title,
            });
        }
        match clean(self.title_concept.as_deref()) {
            Some(concept) if concept != title => Some(format!("{concept} · {title}")),
            _ => Some(title),
        }
    }
}

/// « A & B & A & B » (doublons fréquents dans l'API) → « A & B ».
fn dedup_names(names: &str) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for name in names.split(" & ").map(str::trim) {
        if !name.is_empty() && !seen.iter().any(|s| s.eq_ignore_ascii_case(name)) {
            seen.push(name);
        }
    }
    seen.join(" & ")
}

fn clean(s: Option<&str>) -> Option<String> {
    let s = s?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!s.is_empty()).then_some(s)
}

/// Le plus fin de ce qui passe à `now` : le morceau plutôt que l'émission
/// qui le contient, l'émission entre deux morceaux.
fn on_air(pull: &Pull, now: i64) -> Option<OnAir> {
    let current = |level: &Level| pull.steps.get(level.items.get(level.position)?);
    let mut songs = Songs::None;
    for (depth, level) in pull.levels.iter().enumerate().rev() {
        let Some(step) = current(level) else {
            continue;
        };
        if (step.start..step.end).contains(&now) {
            return Some(OnAir {
                text: step.text()?,
                end: Some(step.end),
                songs,
            });
        }
        // Niveau le plus fin (les morceaux) : sa position dit si un morceau
        // vient de finir ou va commencer.
        if depth + 1 == pull.levels.len() {
            songs = if step.start > now {
                Songs::Next(step.start)
            } else {
                Songs::Ended(step.end)
            };
        }
    }
    None
}

/// Client `livemeta`. Les appels doivent tourner sur le runtime tokio.
pub struct RadioFrance {
    client: reqwest::Client,
}

impl RadioFrance {
    pub fn new() -> Result<Self, RadioError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "Ondes/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/alr51/ondes)"
            ))
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| RadioError::Network(e.to_string()))?;
        Ok(Self { client })
    }

    /// Ce qui passe sur la station `id` à l'instant `now` (secondes Unix).
    pub async fn on_air(&self, id: u32, now: i64) -> Result<Option<OnAir>, RadioError> {
        let response = self
            .client
            .get(format!("{API}/{id}"))
            .send()
            .await
            .map_err(|e| RadioError::Network(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(RadioError::Status(status.as_u16()));
        }
        let pull: Pull = response
            .json()
            .await
            .map_err(|e| RadioError::Decode(e.to_string()))?;
        Ok(on_air(&pull, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn station_ids_from_urls() {
        for (url, id) in [
            ("https://icecast.radiofrance.fr/fip-midfi.mp3", Some(7)),
            (
                "http://icecast.radiofrance.fr/fiprock-hifi.aac?id=radiofrance",
                Some(64),
            ),
            (
                "https://stream.radiofrance.fr/fip/fip_hifi.m3u8?id=radiofrance",
                Some(7),
            ),
            (
                "https://stream.radiofrance.fr/fbnord/fbnord_midfi.m3u8",
                Some(36),
            ),
            ("http://direct.fipradio.fr/live/fip-webradio4.mp3", Some(64)),
            (
                "http://direct.francebleu.fr/live/Fbelsass-midfi.mp3",
                Some(90),
            ),
            (
                "http://direct.franceinter.fr/live/franceinter-midfi.mp3",
                Some(1),
            ),
            (
                "https://icecast.radiofrance.fr/francemusiquelajazz-hifi.aac",
                Some(405),
            ),
            // Pas de métadonnées pour cette webradio.
            (
                "https://stream.radiofrance.fr/fipcultes/fipcultes_hifi.m3u8",
                None,
            ),
            // Même nom, autre diffuseur.
            ("https://example.com/fip-midfi.mp3", None),
            ("https://notradiofrance.fr/fip-midfi.mp3", None),
        ] {
            assert_eq!(station_id(url), id, "{url}");
        }
    }

    fn pull(json: &str) -> Pull {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn current_song() {
        let p = pull(
            r#"{"steps":{
                "a":{"title":"EF","start":100,"end":200,"embedType":"song","authors":"Chanel Beads","performers":""},
                "b":{"title":"A certain peace / When I nap","start":200,"end":450,"embedType":"song","authors":"Nikki Giovanni","composers":"","performers":""}
            },"levels":[{"items":["a","b"],"position":1}],"stationId":7}"#,
        );
        assert_eq!(
            on_air(&p, 300),
            Some(OnAir {
                text: "Nikki Giovanni - A certain peace / When I nap".into(),
                end: Some(450),
                songs: Songs::None,
            })
        );
        // Morceau terminé et liste pas encore à jour : rien plutôt qu'un titre périmé.
        assert_eq!(on_air(&p, 460), None);
    }

    #[test]
    fn classical_uses_composer() {
        let p = pull(
            r#"{"steps":{"a":{"title":"Cantate BWV 82","start":0,"end":10,"embedType":"song",
                "authors":"","composers":"Jean-Sébastien Bach","performers":"Thomas Quasthoff"}},
                "levels":[{"items":["a"],"position":0}]}"#,
        );
        assert_eq!(
            on_air(&p, 5).unwrap().text,
            "Jean-Sébastien Bach - Cantate BWV 82"
        );
    }

    #[test]
    fn duplicate_artists() {
        assert_eq!(
            dedup_names("Brigitte Fontaine & Khan & Brigitte Fontaine & Khan"),
            "Brigitte Fontaine & Khan"
        );
        assert_eq!(dedup_names("Simon & Garfunkel"), "Simon & Garfunkel");
    }

    #[test]
    fn programme_between_songs() {
        // Émission (niveau 0) contenant des morceaux (niveau 1).
        let p = pull(
            r#"{"steps":{
                "show":{"title":"Décodage cérébral","titleConcept":"La Science au labo","start":0,"end":3600,"embedType":"expression","authors":["d499"]},
                "song":{"title":"Gloria","start":100,"end":300,"embedType":"song","authors":"Men At Work"}
            },"levels":[{"items":["show"],"position":0},{"items":["song"],"position":0}]}"#,
        );
        assert_eq!(on_air(&p, 200).unwrap().text, "Men At Work - Gloria");
        let show = on_air(&p, 400).unwrap();
        assert_eq!(show.text, "La Science au labo · Décodage cérébral");
        assert_eq!(show.end, Some(3600));
        assert_eq!(show.songs, Songs::Ended(300));
        assert_eq!(on_air(&p, 50).unwrap().songs, Songs::Next(100));
        // Sans niveau de morceaux : l'émission seule.
        let p = pull(
            r#"{"steps":{"show":{"title":"Le journal","start":0,"end":3600,"embedType":"expression"}},
                "levels":[{"items":["show"],"position":0}]}"#,
        );
        assert_eq!(on_air(&p, 400).unwrap().songs, Songs::None);
    }
}
