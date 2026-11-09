//! Services radio partagés : client Radio Browser et favoris.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use gpui_kit::*;
use ondes_radio::{Favorites, RadioBrowser, RadioFrance, Station};

use crate::runtime::Rt;

/// Client Radio Browser partagé.
#[derive(Clone)]
pub struct Directory(pub Arc<RadioBrowser>);

impl Global for Directory {}

impl Directory {
    pub fn init(cx: &mut App) {
        match RadioBrowser::new() {
            Ok(rb) => cx.set_global(Directory(Arc::new(rb))),
            Err(e) => tracing::error!("client Radio Browser : {e}"),
        }
    }

    pub fn get(cx: &App) -> Option<Arc<RadioBrowser>> {
        cx.try_global::<Directory>().map(|d| d.0.clone())
    }

    /// Signale l'écoute d'une station de l'annuaire (statistiques communautaires).
    pub fn count_click(station: &Station, cx: &App) {
        if let (Some(rb), Some(uuid)) = (Self::get(cx), station.uuid.clone()) {
            drop(Rt::spawn(cx, async move { rb.count_click(&uuid).await }));
        }
    }
}

/// Client des titres en cours de Radio France (API `livemeta`).
#[derive(Clone)]
pub struct RadioFranceApi(pub Arc<RadioFrance>);

impl Global for RadioFranceApi {}

impl RadioFranceApi {
    pub fn init(cx: &mut App) {
        match RadioFrance::new() {
            Ok(rf) => cx.set_global(RadioFranceApi(Arc::new(rf))),
            Err(e) => tracing::error!("client Radio France : {e}"),
        }
    }

    pub fn get(cx: &App) -> Option<Arc<RadioFrance>> {
        cx.try_global::<RadioFranceApi>().map(|d| d.0.clone())
    }
}

/// Favoris : copie en mémoire de la table SQLite, écritures immédiates.
pub struct FavoritesModel {
    store: Option<Favorites>,
    stations: Vec<Station>,
    urls: HashSet<String>,
}

pub struct GlobalFavorites(pub Entity<FavoritesModel>);
impl Global for GlobalFavorites {}

impl FavoritesModel {
    pub fn init(cx: &mut App) {
        let path = crate::paths::database();
        let store = match Favorites::open(&path) {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::error!(path = %path.display(), "favoris indisponibles : {e}");
                None
            }
        };
        let mut model = Self {
            store,
            stations: Vec::new(),
            urls: HashSet::new(),
        };
        model.reload();
        let entity = cx.new(|_| model);
        cx.set_global(GlobalFavorites(entity));
    }

    pub fn global(cx: &App) -> Entity<FavoritesModel> {
        cx.global::<GlobalFavorites>().0.clone()
    }

    fn reload(&mut self) {
        if let Some(store) = &self.store {
            match store.list() {
                Ok(list) => self.stations = list,
                Err(e) => tracing::error!("lecture des favoris : {e}"),
            }
        }
        self.urls = self.stations.iter().map(|s| s.url.clone()).collect();
    }

    pub fn stations(&self) -> &[Station] {
        &self.stations
    }

    pub fn contains(&self, url: &str) -> bool {
        self.urls.contains(url)
    }

    pub fn is_available(&self) -> bool {
        self.store.is_some()
    }

    fn write(
        &mut self,
        f: impl FnOnce(&mut Favorites) -> Result<(), ondes_radio::RadioError>,
    ) -> Result<(), String> {
        let store = self
            .store
            .as_mut()
            .ok_or_else(|| "base de favoris indisponible".to_string())?;
        let result = f(store).map_err(|e| e.to_string());
        self.reload();
        result
    }

    pub fn add(&mut self, station: &Station, cx: &mut Context<Self>) -> Result<(), String> {
        let r = self.write(|s| s.add(station));
        cx.notify();
        r
    }

    pub fn remove(&mut self, url: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let r = self.write(|s| s.remove(url));
        cx.notify();
        r
    }

    /// Ajoute ou retire ; renvoie `true` si la station est désormais favorite.
    pub fn toggle(&mut self, station: &Station, cx: &mut Context<Self>) -> Result<bool, String> {
        if self.contains(&station.url) {
            self.remove(&station.url, cx).map(|_| false)
        } else {
            self.add(station, cx).map(|_| true)
        }
    }

    /// Importe une playlist ; renvoie (stations lues, nouvelles stations).
    pub fn import(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Result<(usize, usize), String> {
        let stations = ondes_radio::import_playlist(path).map_err(|e| e.to_string())?;
        let mut added = 0;
        let r = self.write(|s| {
            added = s.add_all(&stations)?;
            Ok(())
        });
        cx.notify();
        r.map(|_| (stations.len(), added))
    }
}
