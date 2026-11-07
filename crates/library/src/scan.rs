//! Scan incrémental des dossiers de la bibliothèque.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;

use crate::covers::Covers;
use crate::db::{self, FileStamp, LibraryDb};
use crate::{LibraryError, tags};

/// Extensions lues par le moteur audio (symphonia : MP3, AAC/MP4, FLAC, Vorbis, WAV).
const AUDIO_EXTS: &[&str] = &["mp3", "flac", "ogg", "oga", "m4a", "mp4", "aac", "wav"];

/// Taille des lots écrits dans une même transaction.
const BATCH: usize = 200;

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

#[derive(Debug, Default)]
pub struct ScanOptions<'a> {
    /// Interrompt le scan dès que possible.
    pub cancel: Option<&'a AtomicBool>,
}

/// Avancement, pour l'affichage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanProgress {
    /// Parcours des dossiers : `found` fichiers audio vus.
    Listing { found: usize },
    /// Lecture des tags des fichiers nouveaux ou modifiés.
    Reading { done: usize, total: usize },
}

/// Bilan d'un scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    /// Morceaux dans la bibliothèque après le scan.
    pub total: usize,
}

impl ScanReport {
    pub fn changed(&self) -> bool {
        self.added + self.updated + self.removed > 0
    }
}

fn stamp_of(meta: &std::fs::Metadata) -> FileStamp {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    FileStamp {
        mtime,
        size: meta.len(),
    }
}

/// Scanne tous les dossiers de la bibliothèque. Bloquant : à lancer sur un
/// thread dédié. Seuls les fichiers nouveaux ou modifiés sont relus.
pub fn scan(
    db: &mut LibraryDb,
    options: ScanOptions<'_>,
    progress: impl Fn(ScanProgress) + Sync,
) -> Result<ScanReport, LibraryError> {
    let cancelled = || options.cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    let roots = db.folders()?;

    // 1. Inventaire du disque.
    let mut found: HashMap<PathBuf, FileStamp> = HashMap::new();
    for root in &roots {
        for entry in walkdir::WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .flatten()
        {
            if cancelled() {
                return Err(LibraryError::Cancelled);
            }
            if entry.file_type().is_file()
                && is_audio_file(entry.path())
                && let Ok(meta) = entry.metadata()
            {
                found.insert(entry.into_path(), stamp_of(&meta));
                if found.len().is_multiple_of(500) {
                    progress(ScanProgress::Listing { found: found.len() });
                }
            }
        }
    }
    progress(ScanProgress::Listing { found: found.len() });

    // 2. Comparaison avec l'index.
    let known = db.stamps()?;
    let mut to_read: Vec<(PathBuf, FileStamp, bool)> = found
        .iter()
        .filter_map(|(path, stamp)| match known.get(path) {
            Some(old) if old == stamp => None,
            Some(_) => Some((path.clone(), *stamp, true)),
            None => Some((path.clone(), *stamp, false)),
        })
        .collect();
    // Ordre stable : les pochettes de dossier sont trouvées une fois par dossier.
    to_read.sort_by(|a, b| a.0.cmp(&b.0));
    let present: HashSet<&PathBuf> = found.keys().collect();
    let gone: Vec<PathBuf> = known
        .keys()
        .filter(|p| !present.contains(p))
        .cloned()
        .collect();

    // 3. Lecture des tags en parallèle, écriture par lots.
    let covers = Covers::new(db.covers_dir());
    let mut report = ScanReport::default();
    let total = to_read.len();
    progress(ScanProgress::Reading { done: 0, total });
    for (i, batch) in to_read.chunks(BATCH).enumerate() {
        if cancelled() {
            return Err(LibraryError::Cancelled);
        }
        // Tags en parallèle (le plus lent : E/S et analyse des fichiers).
        let mut read: Vec<_> = batch
            .par_iter()
            .map(|(path, stamp, existed)| {
                let mut meta = tags::read(path);
                let album_key = meta.album.as_ref().map(|album| {
                    let artist = meta.album_artist.as_deref().or(meta.artist.as_deref());
                    format!(
                        "{}\u{1f}{}",
                        artist.unwrap_or_default().to_lowercase(),
                        album.to_lowercase()
                    )
                });
                let picture = meta.picture.take();
                (
                    path,
                    *stamp,
                    *existed,
                    meta,
                    album_key,
                    picture,
                    None::<String>,
                )
            })
            .collect();

        // Pochettes dans l'ordre : d'abord les morceaux qui en ont une
        // intégrée, pour qu'elle serve à tout leur album.
        for with_picture in [true, false] {
            for (path, _, _, _, album_key, picture, cover) in &mut read {
                if picture.is_some() == with_picture {
                    *cover = covers.resolve(path, album_key.as_deref(), picture.as_deref());
                    *picture = None;
                }
            }
        }

        let tx = db.transaction()?;
        for (path, stamp, existed, meta, _, _, cover) in &read {
            db::upsert(&tx, path, *stamp, meta, cover.as_deref())?;
            if *existed {
                report.updated += 1;
            } else {
                report.added += 1;
            }
        }
        tx.commit()?;
        progress(ScanProgress::Reading {
            done: (i * BATCH + batch.len()).min(total),
            total,
        });
    }

    // 4. Fichiers disparus (ou dossier retiré).
    if !gone.is_empty() {
        let tx = db.transaction()?;
        for path in &gone {
            db::delete(&tx, path)?;
        }
        tx.commit()?;
        report.removed = gone.len();
    }

    report.total = found.len();
    tracing::info!(?report, "scan terminé");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::config::WriteOptions;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::*;
    use lofty::tag::{Tag, TagType};
    use std::time::Duration;

    fn wav(seconds: u32) -> Vec<u8> {
        let rate = 8_000u32;
        let n = rate * seconds;
        let data_len = n * 2;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&(rate * 2).to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_len.to_le_bytes());
        w.resize(w.len() + data_len as usize, 0);
        w
    }

    fn png() -> Vec<u8> {
        let img = image::DynamicImage::new_rgb8(600, 600);
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn tagged(path: &Path, title: &str, artist: &str, album: &str, track: u32, cover: bool) {
        std::fs::write(path, wav(1)).unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title(title.into());
        tag.set_artist(artist.into());
        tag.set_album(album.into());
        tag.set_track(track);
        if cover {
            tag.push_picture(
                Picture::unchecked(png())
                    .pic_type(PictureType::CoverFront)
                    .mime_type(MimeType::Png)
                    .build(),
            );
        }
        tag.save_to_path(path, WriteOptions::default()).unwrap();
    }

    struct Fixture {
        root: PathBuf,
        music: PathBuf,
        db: LibraryDb,
    }

    fn fixture(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("ondes-scan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let music = root.join("Musique");
        std::fs::create_dir_all(music.join("Album A")).unwrap();
        std::fs::create_dir_all(music.join("Divers")).unwrap();
        let db = LibraryDb::open_in_memory(&root.join("covers")).unwrap();
        db.add_folder(&music).unwrap();
        Fixture { root, music, db }
    }

    #[test]
    fn scans_incrementally() {
        let mut f = fixture("incr");
        let a1 = f.music.join("Album A/01.wav");
        let a2 = f.music.join("Album A/02.wav");
        tagged(&a1, "Un", "Artiste", "Album A", 1, true);
        tagged(&a2, "Deux", "Artiste", "Album A", 2, false);
        std::fs::write(f.music.join("Divers/sans tags.wav"), wav(2)).unwrap();
        std::fs::write(f.music.join("Divers/folder.png"), png()).unwrap();
        std::fs::write(f.music.join("Divers/notes.txt"), "pas audio").unwrap();

        let steps = std::sync::Mutex::new(Vec::new());
        let report = scan(&mut f.db, ScanOptions::default(), |p| {
            steps.lock().unwrap().push(p)
        })
        .unwrap();
        assert_eq!(
            report,
            ScanReport {
                added: 3,
                updated: 0,
                removed: 0,
                total: 3
            }
        );
        assert!(
            steps
                .lock()
                .unwrap()
                .contains(&ScanProgress::Reading { done: 3, total: 3 })
        );

        let mut tracks = f.db.tracks().unwrap();
        tracks.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(tracks[0].title, "Un");
        assert_eq!(tracks[0].track_no, Some(1));
        assert_eq!(tracks[0].duration, Duration::from_secs(1));
        // La pochette intégrée au morceau 1 sert à tout l'album.
        assert!(tracks[0].cover.is_some());
        assert_eq!(tracks[0].cover, tracks[1].cover);
        let thumb = image::open(tracks[0].cover.as_ref().unwrap()).unwrap();
        assert_eq!(thumb.width(), 300);
        // Sans tags : nom du fichier, pochette du dossier.
        assert_eq!(tracks[2].title, "sans tags");
        assert!(tracks[2].cover.is_some());
        assert_ne!(tracks[2].cover, tracks[0].cover);

        // Rien n'a changé : rien n'est relu.
        let report = scan(&mut f.db, ScanOptions::default(), |_| {}).unwrap();
        assert!(!report.changed());

        // Modification, suppression.
        std::thread::sleep(Duration::from_millis(1100));
        tagged(&a2, "Deux (remix)", "Artiste", "Album A", 2, false);
        std::fs::remove_file(&a1).unwrap();
        let report = scan(&mut f.db, ScanOptions::default(), |_| {}).unwrap();
        assert_eq!(
            report,
            ScanReport {
                added: 0,
                updated: 1,
                removed: 1,
                total: 2
            }
        );
        assert!(
            f.db.tracks()
                .unwrap()
                .iter()
                .any(|t| t.title == "Deux (remix)")
        );

        // Retrait du dossier : ses morceaux disparaissent.
        f.db.remove_folder(&f.music).unwrap();
        assert!(f.db.tracks().unwrap().is_empty());
        std::fs::remove_dir_all(&f.root).ok();
    }

    #[test]
    fn honours_cancel() {
        let mut f = fixture("cancel");
        std::fs::write(f.music.join("Divers/a.wav"), wav(1)).unwrap();
        let cancel = AtomicBool::new(true);
        let result = scan(
            &mut f.db,
            ScanOptions {
                cancel: Some(&cancel),
            },
            |_| {},
        );
        assert!(matches!(result, Err(LibraryError::Cancelled)));
        std::fs::remove_dir_all(&f.root).ok();
    }

    #[test]
    fn audio_extensions() {
        assert!(is_audio_file(Path::new("/a/b.FLAC")));
        assert!(is_audio_file(Path::new("x.m4a")));
        assert!(!is_audio_file(Path::new("x.jpg")));
        assert!(!is_audio_file(Path::new("x")));
    }
}
