//! Accès HTTP : cache disque des réponses, cadence de MusicBrainz.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Semaphore};

use crate::InfoError;

/// Durée de validité des réponses en cache.
const TTL: Duration = Duration::from_secs(14 * 24 * 3600);
/// Durée de validité d'une recherche qui n'a rien donné : la fiche peut
/// avoir été ajoutée ou corrigée entre-temps.
const TTL_NOT_FOUND: Duration = Duration::from_secs(24 * 3600);
/// MusicBrainz demande au plus une requête par seconde.
const MB_SPACING: Duration = Duration::from_millis(1100);
/// Nouvelles tentatives après un « serveur occupé » (503). La recherche
/// de MusicBrainz déleste au hasard quand elle est chargée (`Retry-After: 0`) :
/// mieux vaut réessayer souvent et vite que rarement.
const MB_RETRIES: u32 = 7;
/// Attente ajoutée à la cadence, par tentative déjà faite (plafonnée).
const MB_BACKOFF: Duration = Duration::from_millis(400);
const MB_BACKOFF_MAX: Duration = Duration::from_secs(2);
/// Téléchargements d'images simultanés (une discographie en demande des
/// dizaines d'un coup ; Cover Art Archive refuse au-delà).
const IMAGE_DOWNLOADS: usize = 4;

pub(crate) struct Http {
    client: reqwest::Client,
    cache: PathBuf,
    /// Envoi de la dernière requête MusicBrainz ; le verrou les fait passer
    /// une à une.
    mb_last: Mutex<Option<Instant>>,
    images: Semaphore,
}

impl Http {
    pub fn new(cache: PathBuf) -> Result<Self, InfoError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "Ondes/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/alr51/ondes)"
            ))
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| InfoError::Network(e.to_string()))?;
        // Ménage des réponses périmées, sans retarder le démarrage.
        let dir = cache.clone();
        let _ = std::thread::Builder::new()
            .name("ondes-info-cache".into())
            .spawn(move || {
                let removed = purge(&dir);
                if removed > 0 {
                    tracing::debug!(removed, "cache des infos : fichiers périmés supprimés");
                }
            });
        Ok(Self {
            client,
            cache,
            mb_last: Mutex::new(None),
            images: Semaphore::new(IMAGE_DOWNLOADS),
        })
    }

    fn cache_path(&self, url: &str, ext: &str) -> PathBuf {
        let digest = Sha256::digest(url.as_bytes());
        let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
        self.cache.join(format!("{hex}.{ext}"))
    }

    /// JSON d'une API, depuis le cache s'il est récent.
    pub async fn json<T: DeserializeOwned>(&self, url: &str) -> Result<T, InfoError> {
        let path = self.cache_path(url, "json");
        if let Some(bytes) = fresh(&path) {
            match serde_json::from_slice(&bytes) {
                Ok(value) => return Ok(value),
                Err(e) => tracing::debug!(url, "cache illisible : {e}"),
            }
        }
        let bytes = if url.starts_with("https://musicbrainz.org/") {
            self.get_musicbrainz(url).await?
        } else {
            self.get(url).await?
        };
        let value = serde_json::from_slice(&bytes).map_err(|e| InfoError::Decode(e.to_string()))?;
        store(&path, &bytes);
        Ok(value)
    }

    /// Abrège à [`TTL_NOT_FOUND`] la validité de la réponse en cache pour
    /// `url` : une recherche dont rien n'a convenu.
    pub fn expire_soon(&self, url: &str) {
        expire_soon(&self.cache_path(url, "json"));
    }

    /// Image téléchargée dans le cache ; `None` si elle n'existe pas (404),
    /// mémorisé aussi pour ne pas la redemander.
    pub async fn image(&self, url: &str) -> Result<Option<PathBuf>, InfoError> {
        let path = self.cache_path(url, "img");
        let missing = self.cache_path(url, "none");
        if is_fresh(&path) {
            return Ok(Some(path));
        }
        if is_fresh(&missing) {
            return Ok(None);
        }
        let _permit = self
            .images
            .acquire()
            .await
            .map_err(|e| InfoError::Network(e.to_string()))?;
        match self.get(url).await {
            Ok(bytes) => {
                store(&path, &bytes);
                Ok(Some(path))
            }
            Err(InfoError::NotFound) => {
                store(&missing, b"");
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    async fn get(&self, url: &str) -> Result<Vec<u8>, InfoError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| InfoError::Network(e.to_string()))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(InfoError::NotFound);
        }
        if status == reqwest::StatusCode::SERVICE_UNAVAILABLE
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Err(InfoError::Busy);
        }
        if !status.is_success() {
            return Err(InfoError::Network(format!("HTTP {status}")));
        }
        Ok(response
            .bytes()
            .await
            .map_err(|e| InfoError::Network(e.to_string()))?
            .to_vec())
    }

    /// Une requête à la fois, envoyées à [`MB_SPACING`] d'écart au moins.
    async fn get_musicbrainz(&self, url: &str) -> Result<Vec<u8>, InfoError> {
        let mut last = self.mb_last.lock().await;
        let mut attempt = 0;
        loop {
            if let Some(at) = *last {
                let wait = MB_SPACING.saturating_sub(at.elapsed());
                tokio::time::sleep(wait).await;
            }
            // L'écart se compte d'un envoi à l'autre : attendre en plus la
            // réponse ralentirait pour rien. Noté avant l'envoi, il tient
            // aussi quand la fiche est abandonnée en cours de requête.
            *last = Some(Instant::now());
            match self.get(url).await {
                Err(InfoError::Busy) if attempt < MB_RETRIES => {
                    attempt += 1;
                    tracing::debug!(url, attempt, "MusicBrainz occupé, nouvel essai");
                    tokio::time::sleep((MB_BACKOFF * attempt).min(MB_BACKOFF_MAX)).await;
                }
                result => return result,
            }
        }
    }
}

/// Fichier du cache encore valable (sa date suffit, sans le lire).
fn is_fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|modified| {
            SystemTime::now()
                .duration_since(modified)
                .unwrap_or_default()
                < TTL
        })
}

/// Contenu du fichier du cache s'il est encore valable.
fn fresh(path: &Path) -> Option<Vec<u8>> {
    is_fresh(path).then(|| std::fs::read(path).ok()).flatten()
}

/// Vieillit le fichier pour qu'il ne lui reste que [`TTL_NOT_FOUND`] à
/// vivre ; sans effet s'il lui reste déjà moins (relu depuis le cache).
fn expire_soon(path: &Path) {
    let Some(aged) = SystemTime::now().checked_sub(TTL - TTL_NOT_FOUND) else {
        return;
    };
    let Ok(file) = std::fs::File::options().write(true).open(path) else {
        return;
    };
    if file
        .metadata()
        .and_then(|m| m.modified())
        .is_ok_and(|modified| modified > aged)
    {
        let _ = file.set_modified(aged);
    }
}

/// Supprime du cache les fichiers périmés (ils ne seraient plus relus) ;
/// renvoie leur nombre.
fn purge(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            let path = entry.path();
            let ours = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "json" | "img" | "none" | "tmp"));
            let stale = entry
                .metadata()
                .ok()
                .filter(|m| m.is_file())
                .and_then(|m| m.modified().ok())
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .is_some_and(|age| age >= TTL);
            ours && stale && std::fs::remove_file(&path).is_ok()
        })
        .count()
}

fn store(path: &Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Écriture puis renommage : jamais de fichier à moitié écrit.
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ondes-info-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_aged(path: &Path, age: Duration) {
        std::fs::write(path, b"{}").unwrap();
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    fn age(path: &Path) -> Duration {
        let modified = std::fs::metadata(path).unwrap().modified().unwrap();
        SystemTime::now().duration_since(modified).unwrap()
    }

    #[test]
    fn purge_removes_only_stale_cache_files() {
        let dir = temp_dir("purge");
        let day = Duration::from_secs(24 * 3600);
        write_aged(&dir.join("old.json"), TTL + day);
        write_aged(&dir.join("old.img"), TTL + day);
        write_aged(&dir.join("old.none"), TTL + day);
        write_aged(&dir.join("recent.json"), day);
        write_aged(&dir.join("autre.txt"), TTL + day);
        assert_eq!(purge(&dir), 3);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["autre.txt", "recent.json"]);
        assert_eq!(purge(&dir.join("absent")), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn not_found_expires_after_a_day() {
        let dir = temp_dir("expire");
        let path = dir.join("search.json");
        write_aged(&path, Duration::ZERO);
        expire_soon(&path);
        assert!(fresh(&path).is_some());
        let left = TTL - age(&path);
        assert!(left <= TTL_NOT_FOUND && left > TTL_NOT_FOUND - Duration::from_secs(60));

        // Relue depuis le cache le lendemain : l'échéance ne recule pas.
        write_aged(&path, TTL - Duration::from_secs(3600));
        expire_soon(&path);
        assert!(TTL - age(&path) <= Duration::from_secs(3600));
        std::fs::remove_dir_all(dir).ok();
    }
}
