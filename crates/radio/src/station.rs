//! Une station de radio.

use serde::{Deserialize, Serialize};

/// Station, venant de Radio Browser, d'un import ou d'un ajout manuel.
///
/// L'identité d'une station est son URL de flux.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Station {
    /// Identifiant Radio Browser (`stationuuid`), si connu.
    pub uuid: Option<String>,
    pub name: String,
    /// URL de flux (déjà résolue si Radio Browser la fournit).
    pub url: String,
    pub homepage: Option<String>,
    /// URL du logo.
    pub favicon: Option<String>,
    pub country: Option<String>,
    /// Code ISO 3166-1 alpha-2.
    pub countrycode: Option<String>,
    pub languages: Vec<String>,
    pub tags: Vec<String>,
    pub codec: Option<String>,
    /// kbit/s, 0 si inconnu.
    pub bitrate: u32,
}

impl Station {
    /// Station minimale (ajout manuel, import).
    pub fn manual(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            ..Default::default()
        }
    }

    /// Ligne de description courte : « France · MP3 128 kbit/s · jazz, soul ».
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(country) = self.country.as_deref().filter(|c| !c.is_empty()) {
            parts.push(country.to_string());
        }
        match (
            self.codec.as_deref().filter(|c| !c.is_empty()),
            self.bitrate,
        ) {
            (Some(codec), 0) => parts.push(codec.to_string()),
            (Some(codec), b) => parts.push(format!("{codec} {b} kbit/s")),
            (None, 0) => {}
            (None, b) => parts.push(format!("{b} kbit/s")),
        }
        if !self.tags.is_empty() {
            parts.push(
                self.tags
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        parts.join(" · ")
    }
}

/// Découpe une liste « a,b , c » en éléments non vides.
pub(crate) fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary() {
        let s = Station {
            country: Some("France".into()),
            codec: Some("MP3".into()),
            bitrate: 128,
            tags: split_list("jazz, soul,,funk,blues"),
            ..Station::manual("FIP", "http://x")
        };
        assert_eq!(s.summary(), "France · MP3 128 kbit/s · jazz, soul, funk");
        assert_eq!(Station::manual("A", "http://a").summary(), "");
    }
}
