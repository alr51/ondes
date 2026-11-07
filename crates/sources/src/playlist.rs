//! Résolution minimale des playlists de stations (M3U, PLS).
//!
//! Beaucoup de radios publient une URL `.m3u` ou `.pls` qui pointe vers le
//! vrai flux. On ne garde que la première URL http(s). L'import complet de
//! playlists est prévu en phase 2.

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

/// Renvoie la première URL de flux d'une playlist.
pub fn first_stream_url(kind: PlaylistKind, body: &str) -> Result<String, SourceError> {
    let body = body.trim_start_matches('\u{feff}');
    let found = match kind {
        PlaylistKind::M3u => {
            if body.contains("#EXT-X-") {
                return Err(SourceError::HlsUnsupported);
            }
            body.lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with('#') && is_http(l))
                .map(str::to_string)
        }
        PlaylistKind::Pls => body
            .lines()
            .filter_map(|l| {
                let (key, value) = l.split_once('=')?;
                key.trim()
                    .to_ascii_lowercase()
                    .starts_with("file")
                    .then(|| value.trim())
            })
            .find(|v| is_http(v))
            .map(str::to_string),
    };
    found.ok_or(SourceError::EmptyPlaylist)
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
    fn rejects_hls_and_empty() {
        assert!(matches!(
            first_stream_url(PlaylistKind::M3u, "#EXTM3U\n#EXT-X-VERSION:3\nseg.ts\n"),
            Err(SourceError::HlsUnsupported)
        ));
        assert!(matches!(
            first_stream_url(PlaylistKind::Pls, "[playlist]\n"),
            Err(SourceError::EmptyPlaylist)
        ));
    }
}
