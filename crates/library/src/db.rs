//! Index SQLite de la bibliothèque (même fichier que les favoris, tables à part).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, Transaction, params};

use crate::tags::TrackMeta;
use crate::{LibraryError, Track};

/// État connu d'un fichier, pour le scan incrémental.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileStamp {
    pub mtime: i64,
    pub size: u64,
}

pub struct LibraryDb {
    conn: Connection,
    covers_dir: PathBuf,
}

impl LibraryDb {
    /// Ouvre la base ; `covers_dir` contient les miniatures de pochettes.
    pub fn open(path: &Path, covers_dir: &Path) -> Result<Self, LibraryError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?, covers_dir)
    }

    pub fn open_in_memory(covers_dir: &Path) -> Result<Self, LibraryError> {
        Self::init(Connection::open_in_memory()?, covers_dir)
    }

    fn init(conn: Connection, covers_dir: &Path) -> Result<Self, LibraryError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS library_folders (
                id       INTEGER PRIMARY KEY,
                path     TEXT NOT NULL UNIQUE,
                added_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE TABLE IF NOT EXISTS library_tracks (
                id           INTEGER PRIMARY KEY,
                path         TEXT NOT NULL UNIQUE,
                mtime        INTEGER NOT NULL,
                size         INTEGER NOT NULL,
                title        TEXT NOT NULL,
                artist       TEXT,
                album        TEXT,
                album_artist TEXT,
                track_no     INTEGER,
                disc_no      INTEGER,
                year         INTEGER,
                genre        TEXT,
                duration_ms  INTEGER NOT NULL DEFAULT 0,
                cover        TEXT
            );",
        )?;
        Ok(Self {
            conn,
            covers_dir: covers_dir.to_path_buf(),
        })
    }

    pub fn covers_dir(&self) -> &Path {
        &self.covers_dir
    }

    pub fn folders(&self) -> Result<Vec<PathBuf>, LibraryError> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT path FROM library_folders ORDER BY path")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(PathBuf::from)
            .collect())
    }

    pub fn add_folder(&self, path: &Path) -> Result<(), LibraryError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO library_folders (path) VALUES (?1)",
            [path.to_string_lossy()],
        )?;
        Ok(())
    }

    /// Retire le dossier et les morceaux qu'il contient.
    pub fn remove_folder(&mut self, path: &Path) -> Result<(), LibraryError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM library_folders WHERE path = ?1",
            [path.to_string_lossy()],
        )?;
        let prefix = format!("{}/", path.to_string_lossy().trim_end_matches('/'));
        tx.execute(
            "DELETE FROM library_tracks WHERE substr(path, 1, length(?1)) = ?1",
            [prefix],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Tous les morceaux, dans l'ordre album / disque / piste.
    pub fn tracks(&self) -> Result<Vec<Track>, LibraryError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT path, title, artist, album, album_artist, track_no, disc_no,
                    year, genre, duration_ms, cover
             FROM library_tracks",
        )?;
        let covers = &self.covers_dir;
        let rows = stmt.query_map([], |r| {
            Ok(Track {
                path: PathBuf::from(r.get::<_, String>(0)?),
                title: r.get(1)?,
                artist: r.get(2)?,
                album: r.get(3)?,
                album_artist: r.get(4)?,
                track_no: r.get(5)?,
                disc_no: r.get(6)?,
                year: r.get(7)?,
                genre: r.get(8)?,
                duration: Duration::from_millis(r.get::<_, i64>(9)?.max(0) as u64),
                cover: r.get::<_, Option<String>>(10)?.map(|c| covers.join(c)),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub(crate) fn stamps(&self) -> Result<HashMap<PathBuf, FileStamp>, LibraryError> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT path, mtime, size FROM library_tracks")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                PathBuf::from(r.get::<_, String>(0)?),
                FileStamp {
                    mtime: r.get(1)?,
                    size: r.get::<_, i64>(2)? as u64,
                },
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub(crate) fn transaction(&mut self) -> Result<Transaction<'_>, LibraryError> {
        Ok(self.conn.transaction()?)
    }
}

pub(crate) fn upsert(
    tx: &Transaction<'_>,
    path: &Path,
    stamp: FileStamp,
    meta: &TrackMeta,
    cover: Option<&str>,
) -> Result<(), LibraryError> {
    tx.prepare_cached(
        "INSERT INTO library_tracks
            (path, mtime, size, title, artist, album, album_artist, track_no,
             disc_no, year, genre, duration_ms, cover)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(path) DO UPDATE SET
            mtime = excluded.mtime, size = excluded.size, title = excluded.title,
            artist = excluded.artist, album = excluded.album,
            album_artist = excluded.album_artist, track_no = excluded.track_no,
            disc_no = excluded.disc_no, year = excluded.year, genre = excluded.genre,
            duration_ms = excluded.duration_ms, cover = excluded.cover",
    )?
    .execute(params![
        path.to_string_lossy(),
        stamp.mtime,
        stamp.size as i64,
        meta.title,
        meta.artist,
        meta.album,
        meta.album_artist,
        meta.track_no,
        meta.disc_no,
        meta.year,
        meta.genre,
        meta.duration.as_millis() as i64,
        cover,
    ])?;
    Ok(())
}

pub(crate) fn delete(tx: &Transaction<'_>, path: &Path) -> Result<(), LibraryError> {
    tx.prepare_cached("DELETE FROM library_tracks WHERE path = ?1")?
        .execute([path.to_string_lossy()])?;
    Ok(())
}
