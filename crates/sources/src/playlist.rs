//! Résolution minimale des playlists de stations (M3U, PLS).
//!
//! Beaucoup de radios publient une URL `.m3u` ou `.pls` qui pointe vers le
//! vrai flux. Les playlists HLS (`#EXT-X-…`) sont lues par le module `hls`.

use crate::SourceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistKind {
    M3u,
    Pls,
}

impl PlaylistKind {
    /// Détecte une playlist d'après le type MIME ou, à défaut, l'extension.
    pub fn detect(mime: Option<&str>, url: &str) -> Option<Self> {
        if let Some(mime) = mime {
            match mime {
                "audio/x-mpegurl"
                | "audio/mpegurl"
                | "application/x-mpegurl"
                | "application/vnd.apple.mpegurl" => return Some(Self::M3u),
                "audio/x-scpls" | "application/pls+xml" => return Some(Self::Pls),
                m if m.starts_with("audio/") || m == "application/ogg" => return None,
                _ => {}
            }
        }
        let path = url
            .split(['?', '#'])
            .next()
            .unwrap_or(url)
            .to_ascii_lowercase();
        if path.ends_with(".m3u") || path.ends_with(".m3u8") {
            Some(Self::M3u)
        } else if path.ends_with(".pls") {
            Some(Self::Pls)
        } else {
            None
        }
    }
}

/// Une entrée de playlist : URL de flux et titre éventuel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistEntry {
    pub url: String,
    pub title: Option<String>,
}

/// Toutes les entrées http(s) d'une playlist, dans l'ordre.
pub fn parse_entries(kind: PlaylistKind, body: &str) -> Result<Vec<PlaylistEntry>, SourceError> {
    let body = body.trim_start_matches('\u{feff}');
    let entries = match kind {
        PlaylistKind::M3u => {
            if body.contains("#EXT-X-") {
                return Err(SourceError::HlsPlaylist);
            }
            let mut entries = Vec::new();
            let mut title = None;
            for line in body.lines().map(str::trim) {
                if let Some(info) = line.strip_prefix("#EXTINF:") {
                    // `#EXTINF:-1 attr="x",Titre` : le titre suit la première virgule hors guillemets.
                    title = split_extinf_title(info);
                } else if !line.is_empty() && !line.starts_with('#') {
                    if is_http(line) {
                        entries.push(PlaylistEntry {
                            url: line.to_string(),
                            title: title.take(),
                        });
                    } else {
                        title = None;
                    }
                }
            }
            entries
        }
        PlaylistKind::Pls => {
            // `FileN=` et `TitleN=` peuvent arriver dans n'importe quel ordre.
            let mut files: Vec<(u32, String)> = Vec::new();
            let mut titles = std::collections::HashMap::new();
            for line in body.lines() {
                let Some((key, value)) = line.split_once('=') else {
                    continue;
                };
                let key = key.trim().to_ascii_lowercase();
                let value = value.trim();
                if let Some(n) = key.strip_prefix("file") {
                    if is_http(value) {
                        files.push((n.parse().unwrap_or(u32::MAX), value.to_string()));
                    }
                } else if let Some(n) = key.strip_prefix("title")
                    && let Ok(n) = n.parse::<u32>()
                    && !value.is_empty()
                {
                    titles.insert(n, value.to_string());
                }
            }
            files.sort_by_key(|(n, _)| *n);
            files
                .into_iter()
                .map(|(n, url)| PlaylistEntry {
                    url,
                    title: titles.remove(&n),
                })
                .collect()
        }
    };
    Ok(entries)
}

fn split_extinf_title(info: &str) -> Option<String> {
    let mut in_quotes = false;
    for (i, c) in info.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                let title = info[i + 1..].trim();
                return (!title.is_empty()).then(|| title.to_string());
            }
            _ => {}
        }
    }
    None
}

/// Renvoie la première URL de flux d'une playlist.
pub fn first_stream_url(kind: PlaylistKind, body: &str) -> Result<String, SourceError> {
    parse_entries(kind, body)?
        .into_iter()
        .next()
        .map(|e| e.url)
        .ok_or(SourceError::EmptyPlaylist)
}

fn is_http(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_kind() {
        assert_eq!(
            PlaylistKind::detect(Some("audio/x-scpls"), "http://x/listen"),
            Some(PlaylistKind::Pls)
        );
        assert_eq!(
            PlaylistKind::detect(None, "http://x/radio.M3U?token=1"),
            Some(PlaylistKind::M3u)
        );
        // Un vrai flux audio n'est jamais une playlist, même en .m3u.
        assert_eq!(
            PlaylistKind::detect(Some("audio/mpeg"), "http://x/a.m3u"),
            None
        );
        assert_eq!(
            PlaylistKind::detect(Some("text/plain"), "http://x/a.pls"),
            Some(PlaylistKind::Pls)
        );
    }

    #[test]
    fn parses_m3u() {
        let body = "\u{feff}#EXTM3U\n#EXTINF:-1,FIP\n\nhttp://icecast.example/fip.mp3\nhttp://backup/fip.mp3\n";
        assert_eq!(
            first_stream_url(PlaylistKind::M3u, body).unwrap(),
            "http://icecast.example/fip.mp3"
        );
    }

    #[test]
    fn parses_pls() {
        let body = "[playlist]\nNumberOfEntries=2\nFile1=https://a.example/stream\nTitle1=A\nFile2=http://b\n";
        assert_eq!(
            first_stream_url(PlaylistKind::Pls, body).unwrap(),
            "https://a.example/stream"
        );
    }

    #[test]
    fn parses_all_entries_with_titles() {
        let m3u = "#EXTM3U\n#EXTINF:-1 tvg-logo=\"a,b\",FIP Jazz\nhttp://a/jazz\nhttp://a/sans-titre\n#EXTINF:-1,Local\n/tmp/x.mp3\nhttp://a/apres-local\n";
        assert_eq!(
            parse_entries(PlaylistKind::M3u, m3u).unwrap(),
            vec![
                PlaylistEntry {
                    url: "http://a/jazz".into(),
                    title: Some("FIP Jazz".into())
                },
                PlaylistEntry {
                    url: "http://a/sans-titre".into(),
                    title: None
                },
                PlaylistEntry {
                    url: "http://a/apres-local".into(),
                    title: None
                },
            ]
        );
        let pls = "[playlist]\nTitle2=Deux\nFile2=http://b/2\nFile1=http://b/1\nTitle1=Un\n";
        assert_eq!(
            parse_entries(PlaylistKind::Pls, pls).unwrap(),
            vec![
                PlaylistEntry {
                    url: "http://b/1".into(),
                    title: Some("Un".into())
                },
                PlaylistEntry {
                    url: "http://b/2".into(),
                    title: Some("Deux".into())
                },
            ]
        );
    }

    #[test]
    fn rejects_hls_and_empty() {
        assert!(matches!(
            first_stream_url(PlaylistKind::M3u, "#EXTM3U\n#EXT-X-VERSION:3\nseg.ts\n"),
            Err(SourceError::HlsPlaylist)
        ));
        assert!(matches!(
            first_stream_url(PlaylistKind::Pls, "[playlist]\n"),
            Err(SourceError::EmptyPlaylist)
        ));
    }
}
