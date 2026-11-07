//! Miniatures de pochettes : une par album, partagée par ses morceaux.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

const SIZE: u32 = 300;
const FOLDER_IMAGES: &[&str] = &["cover", "folder", "front", "album", "albumart"];
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// Cache des pochettes pendant un scan (partagé entre threads).
pub(crate) struct Covers {
    dir: PathBuf,
    /// Clé d'album → nom de fichier de la miniature.
    by_album: Mutex<HashMap<String, String>>,
    /// Dossier → image de pochette trouvée dans le dossier (ou aucune).
    by_dir: Mutex<HashMap<PathBuf, Option<String>>>,
}

impl Covers {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            by_album: Mutex::default(),
            by_dir: Mutex::default(),
        }
    }

    /// Pochette d'un morceau, par ordre de préférence : pochette déjà connue
    /// de son album, image intégrée au fichier, image du dossier
    /// (`cover.jpg`, `folder.png`…). Renvoie le nom de la miniature.
    pub fn resolve(
        &self,
        track: &Path,
        album_key: Option<&str>,
        embedded: Option<&[u8]>,
    ) -> Option<String> {
        if let Some(key) = album_key
            && let Some(name) = self.by_album.lock().ok()?.get(key).cloned()
        {
            return Some(name);
        }
        if let Some(data) = embedded {
            let cache_key = match album_key {
                Some(key) => format!("album:{key}"),
                None => format!("file:{}", track.display()),
            };
            if let Some(name) = self.store(&cache_key, data) {
                if let Some(key) = album_key {
                    self.by_album
                        .lock()
                        .ok()?
                        .insert(key.to_string(), name.clone());
                }
                return Some(name);
            }
        }
        self.folder_image(track)
    }

    fn folder_image(&self, track: &Path) -> Option<String> {
        let dir = track.parent()?;
        if let Some(found) = self.by_dir.lock().ok()?.get(dir) {
            return found.clone();
        }
        let found = find_folder_image(dir).and_then(|img| {
            let data = std::fs::read(&img).ok()?;
            self.store(&format!("dir:{}", dir.display()), &data)
        });
        self.by_dir
            .lock()
            .ok()?
            .insert(dir.to_path_buf(), found.clone());
        found
    }

    /// Réduit l'image et l'écrit (JPEG) ; réutilise une miniature existante.
    fn store(&self, key: &str, data: &[u8]) -> Option<String> {
        let digest = Sha256::digest(key.as_bytes());
        let name: String = digest[..12].iter().map(|b| format!("{b:02x}")).collect();
        let name = format!("{name}.jpg");
        let path = self.dir.join(&name);
        if path.exists() {
            return Some(name);
        }
        let img = image::load_from_memory(data).ok()?;
        let img = if img.width() > SIZE || img.height() > SIZE {
            img.thumbnail(SIZE, SIZE)
        } else {
            img
        };
        std::fs::create_dir_all(&self.dir).ok()?;
        let tmp = path.with_extension("tmp");
        let file = std::fs::File::create(&tmp).ok()?;
        let mut writer = std::io::BufWriter::new(file);
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 85)
            .encode_image(&img.to_rgb8())
            .ok()?;
        drop(writer);
        std::fs::rename(&tmp, &path).ok()?;
        Some(name)
    }
}

fn find_folder_image(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(usize, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let (Some(stem), Some(ext)) = (path.file_stem(), path.extension()) else {
            continue;
        };
        let stem = stem.to_string_lossy().to_lowercase();
        let ext = ext.to_string_lossy().to_lowercase();
        if !IMAGE_EXTS.contains(&ext.as_str()) {
            continue;
        }
        if let Some(rank) = FOLDER_IMAGES.iter().position(|n| *n == stem)
            && best.as_ref().is_none_or(|(r, _)| rank < *r)
        {
            best = Some((rank, path));
        }
    }
    best.map(|(_, p)| p)
}
