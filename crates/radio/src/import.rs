//! Import d'un fichier playlist (.m3u, .m3u8, .pls) en liste de stations.

use std::path::Path;

use ondes_sources::{PlaylistKind, parse_entries};

use crate::{RadioError, Station};

/// Lit une playlist locale. Les entrées sans titre sont nommées d'après
/// l'hôte de leur URL. Les doublons (même URL) sont retirés.
pub fn import_playlist(path: &Path) -> Result<Vec<Station>, RadioError> {
    let kind =
        PlaylistKind::detect(None, &path.to_string_lossy()).ok_or(RadioError::NotAPlaylist)?;
    let bytes = std::fs::read(path)?;
    let body = String::from_utf8_lossy(&bytes);

    let mut seen = std::collections::HashSet::new();
    Ok(parse_entries(kind, &body)?
        .into_iter()
        .filter(|e| seen.insert(e.url.clone()))
        .map(|e| {
            let name = e
                .title
                .unwrap_or_else(|| ondes_sources::SourceSpec::Url(e.url.clone()).label());
            Station::manual(name, e.url)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_m3u_and_pls() {
        let dir = std::env::temp_dir().join(format!("ondes-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let m3u = dir.join("radios.M3U");
        std::fs::write(
            &m3u,
            "#EXTM3U\n#EXTINF:-1,FIP\nhttps://icecast.radiofrance.fr/fip-midfi.mp3\nhttp://www.exemple.org/flux\nhttps://icecast.radiofrance.fr/fip-midfi.mp3\n",
        )
        .unwrap();
        let stations = import_playlist(&m3u).unwrap();
        assert_eq!(stations.len(), 2);
        assert_eq!(stations[0].name, "FIP");
        assert_eq!(stations[1].name, "exemple.org");

        let pls = dir.join("a.pls");
        std::fs::write(&pls, "[playlist]\nFile1=http://a/1\nTitle1=Une\n").unwrap();
        assert_eq!(import_playlist(&pls).unwrap()[0].name, "Une");

        let txt = dir.join("a.txt");
        std::fs::write(&txt, "http://a").unwrap();
        assert!(matches!(
            import_playlist(&txt),
            Err(RadioError::NotAPlaylist)
        ));

        std::fs::remove_dir_all(&dir).ok();
    }
}
