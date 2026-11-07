//! Fichiers audio locaux.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::{Opened, SourceError};

/// Ouvre un fichier local. Les tags (titre, artiste) arrivent en phase 3.
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
    fn missing_file_is_io_error() {
        assert!(matches!(
            open_file(Path::new("/nonexistent/ondes.mp3")),
            Err(SourceError::Io(_))
        ));
    }
}
