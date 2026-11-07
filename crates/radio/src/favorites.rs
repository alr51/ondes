//! Stations favorites, enregistrées dans SQLite.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::station::split_list;
use crate::{RadioError, Station};

/// Version du schéma (`PRAGMA user_version`).
const SCHEMA_VERSION: i32 = 1;

/// Favoris. Les écritures sont immédiates ; l'ordre est celui d'ajout.
pub struct Favorites {
    conn: Connection,
}

impl Favorites {
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
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS favorite_stations (
                    id          INTEGER PRIMARY KEY,
                    url         TEXT NOT NULL UNIQUE,
                    uuid        TEXT,
                    name        TEXT NOT NULL,
                    homepage    TEXT,
                    favicon     TEXT,
                    country     TEXT,
                    countrycode TEXT,
                    languages   TEXT NOT NULL DEFAULT '',
                    tags        TEXT NOT NULL DEFAULT '',
                    codec       TEXT,
                    bitrate     INTEGER NOT NULL DEFAULT 0,
                    added_at    INTEGER NOT NULL DEFAULT (unixepoch())
                );",
            )?;
        }
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Self { conn })
    }

    pub fn list(&self) -> Result<Vec<Station>, RadioError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT uuid, name, url, homepage, favicon, country, countrycode,
                    languages, tags, codec, bitrate
             FROM favorite_stations ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Station {
                uuid: r.get(0)?,
                name: r.get(1)?,
                url: r.get(2)?,
                homepage: r.get(3)?,
                favicon: r.get(4)?,
                country: r.get(5)?,
                countrycode: r.get(6)?,
                languages: split_list(&r.get::<_, String>(7)?),
                tags: split_list(&r.get::<_, String>(8)?),
                codec: r.get(9)?,
                bitrate: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn contains(&self, url: &str) -> Result<bool, RadioError> {
        Ok(self
            .conn
            .prepare_cached("SELECT 1 FROM favorite_stations WHERE url = ?1")?
            .query_row([url], |_| Ok(()))
            .optional()?
            .is_some())
    }

    /// Ajoute la station, ou met à jour ses informations si l'URL existe déjà.
    pub fn add(&self, s: &Station) -> Result<(), RadioError> {
        self.conn
            .prepare_cached(
                "INSERT INTO favorite_stations
                    (url, uuid, name, homepage, favicon, country, countrycode,
                     languages, tags, codec, bitrate)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(url) DO UPDATE SET
                    uuid = COALESCE(excluded.uuid, uuid),
                    name = excluded.name,
                    homepage = COALESCE(excluded.homepage, homepage),
                    favicon = COALESCE(excluded.favicon, favicon),
                    country = COALESCE(excluded.country, country),
                    countrycode = COALESCE(excluded.countrycode, countrycode),
                    languages = excluded.languages,
                    tags = excluded.tags,
                    codec = COALESCE(excluded.codec, codec),
                    bitrate = excluded.bitrate",
            )?
            .execute(params![
                s.url,
                s.uuid,
                s.name,
                s.homepage,
                s.favicon,
                s.country,
                s.countrycode,
                s.languages.join(","),
                s.tags.join(","),
                s.codec,
                s.bitrate,
            ])?;
        Ok(())
    }

    /// Ajoute plusieurs stations en une transaction ; renvoie le nombre de nouvelles.
    pub fn add_all(&mut self, stations: &[Station]) -> Result<usize, RadioError> {
        let before = self.count()?;
        let tx = self.conn.transaction()?;
        for s in stations {
            tx.prepare_cached(
                "INSERT INTO favorite_stations (url, name, favicon) VALUES (?1, ?2, ?3)
                 ON CONFLICT(url) DO NOTHING",
            )?
            .execute(params![s.url, s.name, s.favicon])?;
        }
        tx.commit()?;
        Ok(self.count()? - before)
    }

    pub fn remove(&self, url: &str) -> Result<(), RadioError> {
        self.conn
            .prepare_cached("DELETE FROM favorite_stations WHERE url = ?1")?
            .execute([url])?;
        Ok(())
    }

    fn count(&self) -> Result<usize, RadioError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM favorite_stations", [], |r| {
                r.get::<_, i64>(0)
            })? as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fip() -> Station {
        Station {
            uuid: Some("u1".into()),
            country: Some("France".into()),
            tags: vec!["jazz".into(), "world".into()],
            bitrate: 128,
            ..Station::manual("FIP", "https://icecast.radiofrance.fr/fip-midfi.mp3")
        }
    }

    #[test]
    fn add_list_remove() {
        let fav = Favorites::open_in_memory().unwrap();
        assert!(fav.list().unwrap().is_empty());

        fav.add(&fip()).unwrap();
        fav.add(&Station::manual("Autre", "http://autre")).unwrap();
        let list = fav.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], fip());
        assert!(fav.contains(&fip().url).unwrap());

        // Ré-ajout : mise à jour sans perdre les infos absentes.
        fav.add(&Station::manual("FIP (renommée)", fip().url))
            .unwrap();
        let list = fav.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "FIP (renommée)");
        assert_eq!(list[0].country.as_deref(), Some("France"));

        fav.remove(&fip().url).unwrap();
        assert!(!fav.contains(&fip().url).unwrap());
        assert_eq!(fav.list().unwrap().len(), 1);
    }

    #[test]
    fn add_all_skips_existing() {
        let mut fav = Favorites::open_in_memory().unwrap();
        fav.add(&fip()).unwrap();
        let added = fav
            .add_all(&[
                Station::manual("FIP doublon", fip().url),
                Station::manual("Nouvelle", "http://nouvelle"),
            ])
            .unwrap();
        assert_eq!(added, 1);
        assert_eq!(fav.list().unwrap()[0].name, "FIP");
    }

    #[test]
    fn persists_on_disk() {
        let path = std::env::temp_dir().join(format!("ondes-fav-{}/db.sqlite", std::process::id()));
        Favorites::open(&path).unwrap().add(&fip()).unwrap();
        assert_eq!(Favorites::open(&path).unwrap().list().unwrap(), vec![fip()]);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
