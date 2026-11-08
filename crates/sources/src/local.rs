//! Fichiers audio locaux.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use lofty::config::ParseOptions;
use lofty::prelude::*;

use crate::{Opened, SourceError};

/// Normalisation du volume (tags `REPLAYGAIN_*`), en dB et en amplitude.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ReplayGain {
    pub track_gain: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_gain: Option<f32>,
    pub album_peak: Option<f32>,
}

impl ReplayGain {
    /// Lit les tags d'un fichier (sans décoder l'audio ni les pochettes).
    pub fn read(path: &Path) -> Option<Self> {
        let file = lofty::probe::Probe::open(path)
            .ok()?
            .options(
                ParseOptions::new()
                    .read_properties(false)
                    .read_cover_art(false),
            )
            .read()
            .ok()?;
        let tag = file.primary_tag().or_else(|| file.first_tag())?;
        let value = |key| tag.get_string(key).and_then(parse_number);
        let rg = Self {
            track_gain: value(ItemKey::ReplayGainTrackGain),
            track_peak: value(ItemKey::ReplayGainTrackPeak),
            album_gain: value(ItemKey::ReplayGainAlbumGain),
            album_peak: value(ItemKey::ReplayGainAlbumPeak),
        };
        (rg.track_gain.is_some() || rg.album_gain.is_some()).then_some(rg)
    }
}

/// `-6.54 dB`, `+1,2 dB`, `0.988` → nombre.
fn parse_number(s: &str) -> Option<f32> {
    let s = s
        .trim()
        .trim_end_matches(|c: char| c.is_alphabetic())
        .trim();
    s.replace(',', ".")
        .trim_start_matches('+')
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
}

/// Ouvre un fichier local.
pub fn open_file(path: &Path) -> Result<Opened, SourceError> {
    let file = File::open(path)?;
    let byte_len = file.metadata().ok().map(|m| m.len());
    let hint = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    Ok(Opened {
        reader: Box::new(BufReader::with_capacity(64 * 1024, file)),
        title: title_from_path(path),
        hint,
        mime: None,
        byte_len,
        live: false,
        replay_gain: ReplayGain::read(path),
        recorder: None,
    })
}

pub(crate) fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn opens_file_with_hint_and_title() {
        let dir = std::env::temp_dir().join(format!("ondes-local-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Mon Morceau.MP3");
        std::fs::write(&path, b"abc").unwrap();

        let mut opened = open_file(&path).unwrap();
        assert_eq!(opened.title, "Mon Morceau");
        assert_eq!(opened.hint.as_deref(), Some("mp3"));
        assert_eq!(opened.byte_len, Some(3));
        assert!(!opened.live);
        let mut buf = String::new();
        opened.reader.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "abc");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_replay_gain_values() {
        assert_eq!(parse_number("-6.54 dB"), Some(-6.54));
        assert_eq!(parse_number("+1,5 dB"), Some(1.5));
        assert_eq!(parse_number(" 0.988707 "), Some(0.988707));
        assert_eq!(parse_number("n/a"), None);
    }

    #[test]
    fn missing_file_is_io_error() {
        assert!(matches!(
            open_file(Path::new("/nonexistent/ondes.mp3")),
            Err(SourceError::Io(_))
        ));
    }
}
