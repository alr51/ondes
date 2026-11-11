//! Biographies : Wikidata donne l'article Wikipédia (en français s'il
//! existe), dont le résumé et l'image viennent de l'API REST de Wikipédia.

use std::collections::HashMap;

use serde::Deserialize;

use crate::Bio;
use crate::http::Http;

/// Langues des articles, par ordre de préférence.
const LANGS: &[&str] = &["fr", "en"];

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Entities {
    entities: HashMap<String, Entity>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Entity {
    sitelinks: HashMap<String, Sitelink>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Sitelink {
    title: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Summary {
    #[serde(rename = "type")]
    kind: String,
    extract: String,
    thumbnail: Option<Thumbnail>,
    content_urls: Option<ContentUrls>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Thumbnail {
    source: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ContentUrls {
    desktop: Option<Page>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Page {
    page: String,
}

/// Résumé de l'article lié à l'entité Wikidata `qid`, s'il y en a un.
pub(crate) async fn bio(http: &Http, qid: &str) -> Option<Bio> {
    let sites = LANGS
        .iter()
        .map(|l| format!("{l}wiki"))
        .collect::<Vec<_>>()
        .join("|");
    let url = format!(
        "https://www.wikidata.org/w/api.php?action=wbgetentities&format=json\
         &props=sitelinks&ids={qid}&sitefilter={sites}"
    );
    let entities: Entities = match http.json(&url).await {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(qid, "Wikidata : {e}");
            return None;
        }
    };
    let entity = entities.entities.get(qid)?;
    for lang in LANGS {
        let Some(link) = entity.sitelinks.get(&format!("{lang}wiki")) else {
            continue;
        };
        let title = link.title.replace(' ', "_");
        let url = format!(
            "https://{lang}.wikipedia.org/api/rest_v1/page/summary/{}",
            encode(&title)
        );
        match http.json::<Summary>(&url).await {
            Ok(s) if s.kind != "disambiguation" && !s.extract.trim().is_empty() => {
                return Some(Bio {
                    extract: s.extract.trim().to_string(),
                    url: s
                        .content_urls
                        .and_then(|c| c.desktop)
                        .map(|d| d.page)
                        .unwrap_or_else(|| format!("https://{lang}.wikipedia.org/wiki/{title}")),
                    image: s.thumbnail.map(|t| t.source).filter(|s| !s.is_empty()),
                    lang: (*lang).to_string(),
                });
            }
            Ok(_) => {}
            Err(e) => tracing::debug!(%url, "Wikipédia : {e}"),
        }
    }
    None
}

/// Encodage d'un titre d'article dans un chemin d'URL.
fn encode(title: &str) -> String {
    let mut out = String::new();
    for b in title.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'(' | b')' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn encodes_titles() {
        assert_eq!(super::encode("Björk_(album)"), "Bj%C3%B6rk_(album)");
        assert_eq!(super::encode("AC/DC"), "AC%2FDC");
    }
}
