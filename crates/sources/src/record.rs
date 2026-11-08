//! Enregistrement d'un flux en direct dans un fichier.
//!
//! Les octets sont copiés tels que le décodeur les reçoit (métadonnées ICY
//! retirées, segments HLS dépaquetés) : pas de réencodage, donc aucune perte.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Poignée d'enregistrement d'une source ; clonable.
#[derive(Clone, Default)]
pub struct Recorder(Arc<Inner>);

#[derive(Default)]
struct Inner {
    active: AtomicBool,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    file: Option<(PathBuf, BufWriter<File>)>,
    /// Octets à écrire en tête de tout enregistrement (initialisation fMP4).
    header: Vec<u8>,
    written: u64,
    error: Option<String>,
}

/// Bilan d'un enregistrement terminé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub path: PathBuf,
    pub bytes: u64,
    pub error: Option<String>,
}

impl Recorder {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Commence à écrire dans `path` (remplacé s'il existe).
    pub fn start(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut file = BufWriter::new(File::create(path)?);
        let mut state = self.state();
        file.write_all(&state.header)?;
        state.written = state.header.len() as u64;
        state.error = None;
        state.file = Some((path.to_path_buf(), file));
        self.0.active.store(true, Ordering::Release);
        Ok(())
    }

    /// Termine l'enregistrement en cours.
    pub fn stop(&self) -> Option<Recorded> {
        self.0.active.store(false, Ordering::Release);
        let mut state = self.state();
        let (path, mut file) = state.file.take()?;
        let mut error = state.error.take();
        if let Err(e) = file.flush()
            && error.is_none()
        {
            error = Some(e.to_string());
        }
        Some(Recorded {
            path,
            bytes: state.written,
            error,
        })
    }

    pub fn is_recording(&self) -> bool {
        self.0.active.load(Ordering::Acquire)
    }

    /// Erreur d'écriture survenue (l'enregistrement est alors suspendu).
    pub fn failed(&self) -> bool {
        self.state().error.is_some()
    }

    pub(crate) fn set_header(&self, header: Vec<u8>) {
        self.state().header = header;
    }

    pub(crate) fn write(&self, data: &[u8]) {
        if data.is_empty() || !self.is_recording() {
            return;
        }
        let mut state = self.state();
        let State {
            file,
            written,
            error,
            ..
        } = &mut *state;
        let Some((_, f)) = file else {
            return;
        };
        if error.is_some() {
            return;
        }
        match f.write_all(data) {
            Ok(()) => *written += data.len() as u64,
            Err(e) => {
                tracing::warn!("enregistrement interrompu : {e}");
                *error = Some(e.to_string());
                self.0.active.store(false, Ordering::Release);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_with_header_between_start_and_stop() {
        let dir = std::env::temp_dir().join(format!("ondes-record-{}", std::process::id()));
        let path = dir.join("sous-dossier/radio.aac");
        let rec = Recorder::default();
        rec.set_header(b"INIT".to_vec());
        rec.write(b"avant");
        rec.start(&path).unwrap();
        assert!(rec.is_recording());
        rec.write(b"-donnees");
        let done = rec.stop().unwrap();
        rec.write(b"apres");
        assert_eq!(done.bytes, 12);
        assert_eq!(done.error, None);
        assert_eq!(std::fs::read(&path).unwrap(), b"INIT-donnees");
        assert!(rec.stop().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
