//! Modèle de la bibliothèque côté UI : dossiers, morceaux en mémoire,
//! scan en arrière-plan et surveillance des dossiers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::*;
use notify::Watcher as _;
use ondes_library::search::{Query, haystack};
use ondes_library::{
    Album, Artist, LibraryDb, ScanOptions, ScanProgress, ScanReport, Track, albums, artists,
    is_audio_file, scan,
};

/// Délai de calme après un changement sur le disque avant de rescanner.
const WATCH_DEBOUNCE: Duration = Duration::from_secs(2);

/// Données dérivées, recalculées après chaque scan (hors du thread UI).
#[derive(Default)]
pub struct Snapshot {
    pub tracks: Vec<Track>,
    /// Texte de recherche de chaque morceau (même indice).
    pub haystacks: Vec<String>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
}

impl Snapshot {
    fn build(tracks: Vec<Track>) -> Self {
        Self {
            haystacks: tracks.iter().map(haystack).collect(),
            albums: albums(&tracks),
            artists: artists(&tracks),
            tracks,
        }
    }

    /// Indices des morceaux de `scope` correspondant à la recherche.
    pub fn filter(&self, scope: impl IntoIterator<Item = usize>, query: &Query) -> Vec<usize> {
        scope
            .into_iter()
            .filter(|&i| {
                query.is_empty() || self.haystacks.get(i).is_some_and(|h| query.matches(h))
            })
            .collect()
    }
}

enum ScanMsg {
    Progress(ScanProgress),
    Done(Result<(ScanReport, Vec<Track>), String>),
}

pub struct LibraryModel {
    db_path: PathBuf,
    covers_dir: PathBuf,
    pub folders: Vec<PathBuf>,
    pub data: Arc<Snapshot>,
    /// Scan en cours et son avancement.
    pub scanning: Option<ScanProgress>,
    pub last_report: Option<ScanReport>,
    pub error: Option<String>,
    rescan_requested: bool,
    cancel: Arc<AtomicBool>,
    watcher: Option<notify::RecommendedWatcher>,
    _debounce: Option<Task<()>>,
    _tasks: Vec<Task<()>>,
}

pub struct GlobalLibrary(pub Entity<LibraryModel>);
impl Global for GlobalLibrary {}

impl LibraryModel {
    pub fn init(cx: &mut App) {
        let entity = cx.new(|cx| {
            let mut model = Self {
                db_path: crate::paths::database(),
                covers_dir: crate::paths::cover_cache(),
                folders: Vec::new(),
                data: Arc::new(Snapshot::default()),
                scanning: None,
                last_report: None,
                error: None,
                rescan_requested: false,
                cancel: Arc::new(AtomicBool::new(false)),
                watcher: None,
                _debounce: None,
                _tasks: Vec::new(),
            };
            model.start_watcher(cx);
            match model.open() {
                Ok(db) => {
                    model.folders = db.folders().unwrap_or_default();
                    // Affiche tout de suite l'index existant, puis rescanne.
                    match db.tracks() {
                        Ok(tracks) => model.install(tracks, cx),
                        Err(e) => model.error = Some(e.to_string()),
                    }
                    model.watch_folders();
                    model.rescan(cx);
                }
                Err(e) => model.error = Some(e),
            }
            model
        });
        cx.set_global(GlobalLibrary(entity));
    }

    pub fn global(cx: &App) -> Entity<LibraryModel> {
        cx.global::<GlobalLibrary>().0.clone()
    }

    fn open(&self) -> Result<LibraryDb, String> {
        LibraryDb::open(&self.db_path, &self.covers_dir).map_err(|e| e.to_string())
    }

    /// Calcule albums, artistes et index de recherche hors du thread UI.
    fn install(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move { Snapshot::build(tracks) });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let snapshot = task.await;
            this.update(cx, |this, cx| {
                this.data = Arc::new(snapshot);
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn add_folder(&mut self, path: &Path, cx: &mut Context<Self>) -> Result<(), String> {
        let db = self.open()?;
        db.add_folder(path).map_err(|e| e.to_string())?;
        self.folders = db.folders().map_err(|e| e.to_string())?;
        self.watch_folders();
        self.rescan(cx);
        cx.notify();
        Ok(())
    }

    pub fn remove_folder(&mut self, path: &Path, cx: &mut Context<Self>) -> Result<(), String> {
        // Un scan en cours pourrait réinsérer les morceaux retirés.
        self.cancel.store(true, Ordering::Relaxed);
        let mut db = self.open()?;
        db.remove_folder(path).map_err(|e| e.to_string())?;
        self.folders = db.folders().map_err(|e| e.to_string())?;
        let tracks = db.tracks().map_err(|e| e.to_string())?;
        self.install(tracks, cx);
        self.watch_folders();
        cx.notify();
        Ok(())
    }

    /// Lance un scan incrémental (ou le reprogramme si un scan tourne déjà).
    pub fn rescan(&mut self, cx: &mut Context<Self>) {
        if self.scanning.is_some() {
            self.rescan_requested = true;
            return;
        }
        if self.folders.is_empty() {
            return;
        }
        self.rescan_requested = false;
        self.error = None;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.scanning = Some(ScanProgress::Listing { found: 0 });
        cx.notify();

        let (tx, rx) = flume::unbounded();
        let (db_path, covers_dir, cancel) = (
            self.db_path.clone(),
            self.covers_dir.clone(),
            self.cancel.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("ondes-scan".into())
            .spawn(move || {
                let progress_tx = tx.clone();
                let result = LibraryDb::open(&db_path, &covers_dir)
                    .and_then(|mut db| {
                        let report = scan(
                            &mut db,
                            ScanOptions {
                                cancel: Some(&cancel),
                            },
                            |p| {
                                let _ = progress_tx.send(ScanMsg::Progress(p));
                            },
                        )?;
                        Ok((report, db.tracks()?))
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(ScanMsg::Done(result));
            });
        if let Err(e) = spawned {
            self.scanning = None;
            self.error = Some(e.to_string());
            return;
        }

        self._tasks.push(cx.spawn(async move |this, cx| {
            while let Ok(msg) = rx.recv_async().await {
                let done = matches!(msg, ScanMsg::Done(_));
                let alive = this
                    .update(cx, |this, cx| {
                        match msg {
                            ScanMsg::Progress(p) => this.scanning = Some(p),
                            ScanMsg::Done(result) => {
                                this.scanning = None;
                                match result {
                                    Ok((report, tracks)) => {
                                        if report.changed()
                                            || this.data.tracks.len() != tracks.len()
                                        {
                                            this.install(tracks, cx);
                                        }
                                        this.last_report = Some(report);
                                    }
                                    Err(e) if e.contains("annulé") => {}
                                    Err(e) => this.error = Some(e),
                                }
                                if this.rescan_requested {
                                    this.rescan(cx);
                                }
                            }
                        }
                        cx.notify();
                    })
                    .is_ok();
                if done || !alive {
                    break;
                }
            }
        }));
    }

    fn start_watcher(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = flume::unbounded::<()>();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            // Fichier audio touché, ou dossier (déplacement / suppression en bloc).
            if event
                .paths
                .iter()
                .any(|p| is_audio_file(p) || p.extension().is_none())
            {
                let _ = tx.send(());
            }
        });
        match watcher {
            Ok(w) => self.watcher = Some(w),
            Err(e) => {
                tracing::warn!("surveillance des dossiers indisponible : {e}");
                return;
            }
        }
        self._tasks.push(cx.spawn(async move |this, cx| {
            while rx.recv_async().await.is_ok() {
                let alive = this
                    .update(cx, |this, cx| {
                        // Regroupe les rafales d'événements (copie d'un album…).
                        this._debounce = Some(cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(WATCH_DEBOUNCE).await;
                            this.update(cx, |this, cx| this.rescan(cx)).ok();
                        }));
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        }));
    }

    fn watch_folders(&mut self) {
        let Some(watcher) = &mut self.watcher else {
            return;
        };
        for folder in &self.folders {
            // Re-surveiller un dossier déjà suivi est sans effet.
            if let Err(e) = watcher.watch(folder, notify::RecursiveMode::Recursive) {
                tracing::warn!(folder = %folder.display(), "surveillance impossible : {e}");
            }
        }
    }

    /// Nombre de morceaux sous un dossier racine.
    pub fn count_in(&self, folder: &Path) -> usize {
        self.data
            .tracks
            .iter()
            .filter(|t| t.path.starts_with(folder))
            .count()
    }
}
