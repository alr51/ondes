//! Client de l'API Radio Browser (<https://www.radio-browser.info>).
//!
//! Annuaire communautaire gratuit, sans clé. Les fonctions sont `async` et
//! doivent tourner sur le runtime tokio (reqwest).

use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;

use crate::station::split_list;
use crate::{RadioError, Station};

/// Serveurs de repli si la découverte échoue.
const FALLBACK_SERVERS: &[&str] = &[
    "de1.api.radio-browser.info",
    "fi1.api.radio-browser.info",
    "de2.api.radio-browser.info",
    "nl1.api.radio-browser.info",
];

/// Critères de recherche. Champs vides = pas de filtre.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub name: String,
    /// Code pays ISO (ex. `FR`).
    pub countrycode: Option<String>,
    /// Nom de langue tel que renvoyé par l'API (ex. `french`).
    pub language: Option<String>,
    pub tag: Option<String>,
    pub offset: usize,
    pub limit: usize,
}

impl SearchQuery {
    pub(crate) fn params(&self) -> Vec<(&'static str, String)> {
        let mut p = vec![
            ("hidebroken", "true".to_string()),
            ("order", "clickcount".to_string()),
            ("reverse", "true".to_string()),
            ("offset", self.offset.to_string()),
            ("limit", self.limit.max(1).to_string()),
        ];
        let name = self.name.trim();
        if !name.is_empty() {
            p.push(("name", name.to_string()));
        }
        if let Some(c) = self.countrycode.as_deref().filter(|c| !c.is_empty()) {
            p.push(("countrycode", c.to_string()));
        }
        if let Some(l) = self.language.as_deref().filter(|l| !l.is_empty()) {
            p.push(("language", l.to_string()));
            p.push(("languageExact", "true".to_string()));
        }
        if let Some(t) = self.tag.as_deref().filter(|t| !t.is_empty()) {
            p.push(("tag", t.to_string()));
            p.push(("tagExact", "true".to_string()));
        }
        p
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Country {
    pub name: String,
    #[serde(rename = "iso_3166_1")]
    pub code: String,
    pub stationcount: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Language {
    pub name: String,
    pub stationcount: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Tag {
    pub name: String,
    pub stationcount: u32,
}

/// Station telle que renvoyée par l'API.
#[derive(Debug, Deserialize)]
struct ApiStation {
    stationuuid: String,
    name: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    url_resolved: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    favicon: String,
    #[serde(default)]
    country: String,
    #[serde(default)]
    countrycode: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    tags: String,
    #[serde(default)]
    codec: String,
    #[serde(default)]
    bitrate: u32,
}

fn non_empty(s: String) -> Option<String> {
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

impl From<ApiStation> for Station {
    fn from(a: ApiStation) -> Self {
        let url = if a.url_resolved.trim().is_empty() {
            a.url
        } else {
            a.url_resolved
        };
        Station {
            uuid: non_empty(a.stationuuid),
            name: a.name.trim().to_string(),
            url: url.trim().to_string(),
            homepage: non_empty(a.homepage),
            favicon: non_empty(a.favicon).filter(|f| f.starts_with("http")),
            country: non_empty(a.country),
            countrycode: non_empty(a.countrycode),
            languages: split_list(&a.language),
            tags: split_list(&a.tags),
            codec: non_empty(a.codec).filter(|c| c != "UNKNOWN"),
            bitrate: a.bitrate,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Server {
    name: String,
}

/// Client Radio Browser avec bascule automatique entre serveurs miroirs.
pub struct RadioBrowser {
    client: reqwest::Client,
    servers: Mutex<Vec<String>>,
}

impl RadioBrowser {
    pub fn new() -> Result<Self, RadioError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Ondes/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| RadioError::Network(e.to_string()))?;
        Ok(Self {
            client,
            servers: Mutex::new(Vec::new()),
        })
    }

    /// Liste des miroirs, découverte au premier appel.
    async fn servers(&self) -> Vec<String> {
        if let Ok(s) = self.servers.lock()
            && !s.is_empty()
        {
            return s.clone();
        }
        let mut found: Vec<String> = match self
            .client
            .get("https://all.api.radio-browser.info/json/servers")
            .send()
            .await
        {
            Ok(resp) => resp
                .json::<Vec<Server>>()
                .await
                .map(|v| v.into_iter().map(|s| s.name).collect())
                .unwrap_or_default(),
            Err(e) => {
                tracing::debug!(error = %e, "découverte des serveurs impossible");
                Vec::new()
            }
        };
        found.sort();
        found.dedup();
        if found.is_empty() {
            found = FALLBACK_SERVERS.iter().map(|s| s.to_string()).collect();
        }
        // Répartit la charge entre les miroirs, comme le recommande l'API.
        let start = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as usize)
            .unwrap_or(0)
            % found.len();
        found.rotate_left(start);
        if let Ok(mut s) = self.servers.lock() {
            *s = found.clone();
        }
        found
    }

    /// GET sur le premier miroir qui répond ; le miroir en panne passe en fin de liste.
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<T, RadioError> {
        let servers = self.servers().await;
        let mut last_error = String::from("aucun serveur");
        for server in servers.iter().take(3) {
            let url = format!("https://{server}{path}");
            let result = async {
                let resp = self
                    .client
                    .get(&url)
                    .query(params)
                    .send()
                    .await
                    .map_err(|e| RadioError::Network(e.to_string()))?
                    .error_for_status()
                    .map_err(|e| RadioError::Network(e.to_string()))?;
                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| RadioError::Network(e.to_string()))?;
                serde_json::from_slice::<T>(&bytes).map_err(|e| RadioError::Decode(e.to_string()))
            }
            .await;
            match result {
                Ok(v) => return Ok(v),
                Err(RadioError::Decode(e)) => return Err(RadioError::Decode(e)),
                Err(e) => {
                    tracing::warn!(%server, error = %e, "miroir Radio Browser en échec");
                    last_error = e.to_string();
                    if let Ok(mut s) = self.servers.lock()
                        && let Some(i) = s.iter().position(|x| x == server)
                    {
                        let bad = s.remove(i);
                        s.push(bad);
                    }
                }
            }
        }
        Err(RadioError::Network(last_error))
    }

    pub async fn search(&self, query: &SearchQuery) -> Result<Vec<Station>, RadioError> {
        let stations: Vec<ApiStation> = self.get("/json/stations/search", &query.params()).await?;
        Ok(stations
            .into_iter()
            .map(Station::from)
            .filter(|s| s.url.starts_with("http"))
            .collect())
    }

    pub async fn countries(&self) -> Result<Vec<Country>, RadioError> {
        let mut list: Vec<Country> = self
            .get(
                "/json/countries",
                &[("hidebroken", "true".into()), ("order", "name".into())],
            )
            .await?;
        list.retain(|c| !c.code.is_empty() && c.stationcount > 0);
        Ok(list)
    }

    pub async fn languages(&self) -> Result<Vec<Language>, RadioError> {
        let mut list: Vec<Language> = self
            .get(
                "/json/languages",
                &[
                    ("hidebroken", "true".into()),
                    ("order", "stationcount".into()),
                    ("reverse", "true".into()),
                    ("limit", "150".into()),
                ],
            )
            .await?;
        list.retain(|l| !l.name.is_empty());
        Ok(list)
    }

    pub async fn tags(&self) -> Result<Vec<Tag>, RadioError> {
        let mut list: Vec<Tag> = self
            .get(
                "/json/tags",
                &[
                    ("hidebroken", "true".into()),
                    ("order", "stationcount".into()),
                    ("reverse", "true".into()),
                    ("limit", "300".into()),
                ],
            )
            .await?;
        list.retain(|t| !t.name.is_empty());
        Ok(list)
    }

    /// Signale une écoute (statistiques de l'annuaire, recommandé par l'API).
    pub async fn count_click(&self, uuid: &str) {
        let path = format!("/json/url/{uuid}");
        if let Err(e) = self.get::<serde_json::Value>(&path, &[]).await {
            tracing::debug!(error = %e, "comptage d'écoute ignoré");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_api_station() {
        let json = r#"[{
            "changeuuid": "x", "stationuuid": "96202f73-0601-11e8-ae97-52543be04c81",
            "name": " FIP ", "url": "http://direct.fip.fr/live/fip-midfi.mp3",
            "url_resolved": "https://icecast.radiofrance.fr/fip-midfi.mp3",
            "homepage": "https://www.fip.fr/", "favicon": "https://www.radiofrance.fr/fip.png",
            "tags": "jazz,eclectic,world", "country": "France", "countrycode": "FR",
            "language": "french", "votes": 1000, "codec": "MP3", "bitrate": 128,
            "lastcheckok": 1, "clickcount": 500
        }, {
            "stationuuid": "y", "name": "Sans logo", "url": "http://a/b",
            "favicon": "data:image/png;base64,xx", "codec": "UNKNOWN", "bitrate": 0
        }]"#;
        let stations: Vec<Station> = serde_json::from_str::<Vec<ApiStation>>(json)
            .unwrap()
            .into_iter()
            .map(Station::from)
            .collect();
        let fip = &stations[0];
        assert_eq!(fip.name, "FIP");
        assert_eq!(fip.url, "https://icecast.radiofrance.fr/fip-midfi.mp3");
        assert_eq!(fip.tags, vec!["jazz", "eclectic", "world"]);
        assert_eq!(fip.languages, vec!["french"]);
        assert_eq!(fip.countrycode.as_deref(), Some("FR"));

        let other = &stations[1];
        assert_eq!(other.url, "http://a/b");
        assert_eq!(other.favicon, None);
        assert_eq!(other.codec, None);
    }

    #[test]
    fn query_params() {
        let q = SearchQuery {
            name: "  fip ".into(),
            countrycode: Some("FR".into()),
            language: Some(String::new()),
            tag: Some("jazz".into()),
            offset: 100,
            limit: 50,
        };
        let p = q.params();
        let get = |k: &str| p.iter().find(|(kk, _)| *kk == k).map(|(_, v)| v.as_str());
        assert_eq!(get("name"), Some("fip"));
        assert_eq!(get("countrycode"), Some("FR"));
        assert_eq!(get("language"), None);
        assert_eq!(get("tagExact"), Some("true"));
        assert_eq!(get("offset"), Some("100"));
        assert_eq!(get("limit"), Some("50"));
    }

    /// Appel réel à l'API : `cargo test -p ondes-radio -- --ignored`.
    #[test]
    #[ignore = "réseau"]
    fn live_search() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let rb = RadioBrowser::new().unwrap();
        let found = rt
            .block_on(rb.search(&SearchQuery {
                name: "fip".into(),
                countrycode: Some("FR".into()),
                limit: 10,
                ..Default::default()
            }))
            .unwrap();
        assert!(!found.is_empty());
        assert!(rt.block_on(rb.countries()).unwrap().len() > 50);
    }
}
