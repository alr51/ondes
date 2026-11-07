//! Logos de stations : téléchargés à la demande (lignes visibles seulement),
//! réduits à 96 px et gardés en cache disque.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::runtime::Rt;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const SIZE: u32 = 96;
const PARALLEL: usize = 6;

#[derive(Clone)]
enum State {
    Pending,
    Ready(PathBuf),
    Failed,
}

pub struct Logos {
    dir: PathBuf,
    client: reqwest::Client,
    limit: Arc<Semaphore>,
    states: HashMap<String, State>,
}

impl Global for GlobalLogos {}
pub struct GlobalLogos(pub Entity<Logos>);

impl Logos {
    pub fn init(cx: &mut App) {
        let dir = crate::paths::logo_cache();
        let client = reqwest::Client::builder()
            .user_agent(concat!("Ondes/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .build()
            .unwrap_or_default();
        let entity = cx.new(|_| Self {
            dir,
            client,
            limit: Arc::new(Semaphore::new(PARALLEL)),
            states: HashMap::new(),
        });
        cx.set_global(GlobalLogos(entity));
    }

    pub fn global(cx: &App) -> Entity<Logos> {
        cx.global::<GlobalLogos>().0.clone()
    }

    /// Chemin local du logo s'il est prêt ; lance sinon son téléchargement.
    pub fn get(&mut self, url: &str, cx: &mut Context<Self>) -> Option<PathBuf> {
        match self.states.get(url) {
            Some(State::Ready(path)) => return Some(path.clone()),
            Some(State::Pending | State::Failed) => return None,
            None => {}
        }
        let path = self.dir.join(cache_name(url));
        if path.exists() {
            self.states
                .insert(url.to_string(), State::Ready(path.clone()));
            return Some(path);
        }
        self.states.insert(url.to_string(), State::Pending);

        let task = Rt::spawn(
            cx,
            fetch(
                self.client.clone(),
                self.limit.clone(),
                url.to_string(),
                path.clone(),
            ),
        );
        let url = url.to_string();
        cx.spawn(async move |this, cx| {
            let state = match task.await {
                Ok(Ok(())) => State::Ready(path),
                Ok(Err(e)) => {
                    tracing::debug!(%url, error = %e, "logo indisponible");
                    State::Failed
                }
                Err(_) => State::Failed,
            };
            this.update(cx, |this, cx| {
                this.states.insert(url, state);
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }
}

fn cache_name(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}.img")
}

async fn fetch(
    client: reqwest::Client,
    limit: Arc<Semaphore>,
    url: String,
    dest: PathBuf,
) -> Result<(), String> {
    let _permit = limit.acquire_owned().await.map_err(|e| e.to_string())?;
    let mut resp = client
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_BYTES {
            return Err("logo trop lourd".into());
        }
    }
    tokio::task::spawn_blocking(move || store(&bytes, &dest))
        .await
        .map_err(|e| e.to_string())?
}

/// Réduit l'image et l'écrit en PNG ; les SVG sont gardés tels quels.
fn store(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let out = match image::load_from_memory(bytes) {
        Ok(img) => {
            let img = if img.width() > SIZE || img.height() > SIZE {
                img.thumbnail(SIZE, SIZE)
            } else {
                img
            };
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            png
        }
        Err(e) => {
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_lowercase();
            if head.contains("<svg") {
                bytes.to_vec()
            } else {
                return Err(format!("image illisible : {e}"));
            }
        }
    };
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Écriture atomique : jamais de fichier à moitié écrit dans le cache.
    let tmp = dest.with_extension("tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    // Pas de `super::*` : `gpui_kit::*` exporte sa propre macro `test`.
    use super::{cache_name, store};

    #[test]
    fn stores_thumbnails_and_svg() {
        let dir = std::env::temp_dir().join(format!("ondes-logos-{}", std::process::id()));
        let big = image::DynamicImage::new_rgb8(400, 200);
        let mut png = Vec::new();
        big.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let dest = dir.join(cache_name("http://a/logo.png"));
        store(&png, &dest).unwrap();
        // Format deviné d'après le contenu, comme le fait GPUI.
        let small = image::ImageReader::open(&dest)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!((small.width(), small.height()), (96, 48));

        let svg = dir.join("x.img");
        store(
            b"<?xml version='1.0'?><svg xmlns='http://www.w3.org/2000/svg'/>",
            &svg,
        )
        .unwrap();
        assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));

        assert!(store(b"<html>pas une image</html>", &dir.join("y.img")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cache_names_are_stable() {
        assert_eq!(cache_name("http://a"), cache_name("http://a"));
        assert_ne!(cache_name("http://a"), cache_name("http://b"));
        assert_eq!(cache_name("http://a").len(), 36);
    }
}
