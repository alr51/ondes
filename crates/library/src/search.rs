//! Recherche en mémoire : tous les mots doivent apparaître, sans tenir
//! compte de la casse ni des accents (« beyonce » trouve « Beyoncé »).

use crate::Track;

/// Minuscules sans accents (alphabets latins courants).
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => out.push('a'),
            'æ' => out.push_str("ae"),
            'ç' | 'ć' | 'č' => out.push('c'),
            'ď' | 'đ' => out.push('d'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => out.push('i'),
            'ł' | 'ľ' | 'ĺ' => out.push('l'),
            'ñ' | 'ń' | 'ň' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => out.push('o'),
            'œ' => out.push_str("oe"),
            'ŕ' | 'ř' => out.push('r'),
            'ś' | 'š' | 'ş' => out.push('s'),
            'ß' => out.push_str("ss"),
            'ť' | 'ţ' => out.push('t'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'ų' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ź' | 'ż' | 'ž' => out.push('z'),
            c => out.push(c),
        }
    }
    out
}

/// Texte de recherche d'un morceau (à calculer une fois par morceau).
pub fn haystack(track: &Track) -> String {
    let file = track
        .path
        .file_name()
        .map(|f| f.to_string_lossy())
        .unwrap_or_default();
    let parts = [
        Some(track.title.as_str()),
        track.artist.as_deref(),
        track.album.as_deref(),
        track.album_artist.as_deref(),
        track.genre.as_deref(),
        Some(&*file),
    ];
    fold(
        &parts
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Requête découpée en mots repliés.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query(Vec<String>);

impl Query {
    pub fn new(text: &str) -> Self {
        Self(fold(text).split_whitespace().map(str::to_string).collect())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn matches(&self, haystack: &str) -> bool {
        self.0.iter().all(|word| haystack.contains(word.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_accents_and_case() {
        assert_eq!(fold("Beyoncé – Œuvre ÇA"), "beyonce – oeuvre ca");
    }

    #[test]
    fn all_words_must_match() {
        let t = Track {
            title: "Déjà Vu".into(),
            artist: Some("Beyoncé".into()),
            album: Some("B'Day".into()),
            path: "/m/b/02 Deja Vu.flac".into(),
            ..Default::default()
        };
        let h = haystack(&t);
        assert!(Query::new("beyonce deja").matches(&h));
        assert!(Query::new("  BDAY ").is_empty() || !Query::new("BDAY").matches(&h));
        assert!(Query::new("b'day vu").matches(&h));
        assert!(Query::new("flac").matches(&h));
        assert!(!Query::new("beyonce halo").matches(&h));
        assert!(Query::new("").matches(&h));
    }
}
