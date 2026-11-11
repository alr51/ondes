//! Historique d'écoute : titres annoncés par les radios et morceaux joués,
//! dans la même base SQLite que les favoris.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::RadioError;

/// Entrées conservées : les plus anciennes sont effacées au-delà.
const KEEP: usize = 10_000;

/// Même titre sur la même source dans cet intervalle : une seule entrée
/// (reconnexion d'un direct, morceau relancé).
const DEDUP_SECS: i64 = 30 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryKind {
    /// Titre annoncé par une radio (métadonnées ICY).
    Radio,
    /// Fichier local, de la bibliothèque ou ouvert directement.
    File,
}

impl HistoryKind {
    fn code(self) -> i64 {
        match self {
            HistoryKind::Radio => 0,
            HistoryKind::File => 1,
        }
    }

    fn from_code(code: i64) -> Self {
        if code == 1 {
            HistoryKind::File
        } else {
            HistoryKind::Radio
        }
    }
}

/// Une écoute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: i64,
    /// Secondes Unix.
    pub played_at: i64,
    pub kind: HistoryKind,
    pub title: String,
    pub artist: Option<String>,
    /// Nom de la station, ou album.
    pub source: Option<String>,
    /// URL du flux, ou chemin du fichier.
    pub location: String,
    /// Logo de la station.
    pub favicon: Option<String>,
}

impl HistoryEntry {
    /// Entrée d'un titre annoncé par une radio, souvent « Artiste - Titre ».
    pub fn radio(
        played_at: i64,
        announced: &str,
        station: &str,
        url: &str,
        favicon: Option<String>,
    ) -> Self {
        let (artist, title) = split_announced(announced);
        Self {
            id: 0,
            played_at,
            kind: HistoryKind::Radio,
            title,
            artist,
            source: Some(station.to_string()).filter(|s| !s.is_empty()),
            location: url.to_string(),
            favicon,
        }
    }

    /// « Artiste - Titre » tel qu'annoncé, pour copier ou chercher ailleurs.
    pub fn display_title(&self) -> String {
        match &self.artist {
            Some(artist) => format!("{artist} - {}", self.title),
            None => self.title.clone(),
        }
    }

    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            played_at: r.get(1)?,
            kind: HistoryKind::from_code(r.get(2)?),
            title: r.get(3)?,
            artist: r.get(4)?,
            source: r.get(5)?,
            location: r.get(6)?,
            favicon: r.get(7)?,
        })
    }
}

/// Sépare « Artiste - Titre » ; le titre seul s'il n'y a pas de séparateur.
fn split_announced(text: &str) -> (Option<String>, String) {
    let text = text.trim();
    match text.split_once(" - ") {
        Some((artist, title)) if !artist.trim().is_empty() && !title.trim().is_empty() => {
            (Some(artist.trim().to_string()), title.trim().to_string())
        }
        _ => (None, text.to_string()),
    }
}

pub struct History {
    conn: Connection,
}

impl History {
    /// Ouvre (ou crée) la base à `path`. Les dossiers parents sont créés.
    pub fn open(path: &Path) -> Result<Self, RadioError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, RadioError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, RadioError> {
        // Pas de `user_version` ici : il appartient au schéma des favoris.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS history (
                id        INTEGER PRIMARY KEY,
                played_at INTEGER NOT NULL,
                kind      INTEGER NOT NULL,
                title     TEXT NOT NULL,
                artist    TEXT,
                source    TEXT,
                location  TEXT NOT NULL,
                favicon   TEXT
            );
            CREATE INDEX IF NOT EXISTS history_played_at ON history (played_at);
            -- Dernière écoute d'une source, pour le dédoublonnage.
            CREATE INDEX IF NOT EXISTS history_location ON history (location, id);",
        )?;
        let history = Self { conn };
        history.prune()?;
        Ok(history)
    }

    /// Ajoute l'écoute, sauf si elle répète la précédente de la même source.
    /// Renvoie l'entrée enregistrée.
    pub fn add(&self, entry: &HistoryEntry) -> Result<Option<HistoryEntry>, RadioError> {
        let previous: Option<(String, Option<String>, i64)> = self
            .conn
            .prepare_cached(
                "SELECT title, artist, played_at FROM history
                 WHERE location = ?1 ORDER BY id DESC LIMIT 1",
            )?
            .query_row([&entry.location], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        if let Some((title, artist, at)) = previous
            && title == entry.title
            && artist == entry.artist
            && (entry.played_at - at).abs() < DEDUP_SECS
        {
            return Ok(None);
        }
        self.conn
            .prepare_cached(
                "INSERT INTO history
                    (played_at, kind, title, artist, source, location, favicon)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                entry.played_at,
                entry.kind.code(),
                entry.title,
                entry.artist,
                entry.source,
                entry.location,
                entry.favicon,
            ])?;
        Ok(Some(HistoryEntry {
            id: self.conn.last_insert_rowid(),
            ..entry.clone()
        }))
    }

    /// Les plus récentes d'abord.
    pub fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>, RadioError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, played_at, kind, title, artist, source, location, favicon
             FROM history ORDER BY played_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], HistoryEntry::from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn remove(&self, id: i64) -> Result<(), RadioError> {
        self.conn
            .prepare_cached("DELETE FROM history WHERE id = ?1")?
            .execute([id])?;
        Ok(())
    }

    pub fn clear(&self) -> Result<(), RadioError> {
        self.conn.execute("DELETE FROM history", [])?;
        Ok(())
    }

    fn prune(&self) -> Result<(), RadioError> {
        self.conn.execute(
            "DELETE FROM history WHERE id NOT IN
                (SELECT id FROM history ORDER BY played_at DESC, id DESC LIMIT ?1)",
            [KEEP as i64],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIP: &str = "https://icecast.radiofrance.fr/fip-midfi.mp3";

    #[test]
    fn splits_artist_and_title() {
        let e = HistoryEntry::radio(0, "Nina Simone - Sinnerman ", "FIP", FIP, None);
        assert_eq!(e.artist.as_deref(), Some("Nina Simone"));
        assert_eq!(e.title, "Sinnerman");
        assert_eq!(e.display_title(), "Nina Simone - Sinnerman");

        let e = HistoryEntry::radio(0, "Le journal", "", FIP, None);
        assert_eq!(e.artist, None);
        assert_eq!(e.title, "Le journal");
        assert_eq!(e.source, None);

        // Tiret sans espaces : fait partie du titre.
        let e = HistoryEntry::radio(0, "Jay-Z", "FIP", FIP, None);
        assert_eq!(e.artist, None);
    }

    #[test]
    fn add_dedup_recent_remove_clear() {
        let h = History::open_in_memory().unwrap();
        let a = HistoryEntry::radio(1_000, "A - Un", "FIP", FIP, None);
        let b = HistoryEntry::radio(1_200, "B - Deux", "FIP", FIP, None);

        assert!(h.add(&a).unwrap().is_some());
        // Reconnexion : même titre annoncé de nouveau, ignoré.
        assert!(
            h.add(&HistoryEntry {
                played_at: 1_100,
                ..a.clone()
            })
            .unwrap()
            .is_none()
        );
        let b = h.add(&b).unwrap().unwrap();
        // Retour du premier titre plus tard : nouvelle entrée.
        assert!(
            h.add(&HistoryEntry {
                played_at: 1_000 + DEDUP_SECS + 1,
                ..a.clone()
            })
            .unwrap()
            .is_some()
        );

        let list = h.recent(10).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].title, "Deux");
        assert_eq!(list[2].played_at, 1_000);

        h.remove(b.id).unwrap();
        assert_eq!(h.recent(10).unwrap().len(), 2);
        h.clear().unwrap();
        assert!(h.recent(10).unwrap().is_empty());
    }

    #[test]
    fn keeps_most_recent() {
        let h = History::open_in_memory().unwrap();
        for i in 0..(KEEP as i64 + 5) {
            h.add(&HistoryEntry::radio(
                i * 10_000,
                &format!("t{i}"),
                "",
                FIP,
                None,
            ))
            .unwrap();
        }
        h.prune().unwrap();
        let list = h.recent(KEEP + 10).unwrap();
        assert_eq!(list.len(), KEEP);
        assert_eq!(list.last().unwrap().title, "t5");
    }
}
