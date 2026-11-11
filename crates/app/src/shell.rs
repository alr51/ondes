//! Fenêtre principale : barre latérale, contenu, barre de lecture.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::component_traits::Collapsible as _;
use gpui_kit::component::{
    ActiveTheme, Selectable as _, Sizable as _, StyledExt, Theme, ThemeMode, WindowExt as _,
    button::{Button, ButtonVariants},
    command::{Command, CommandGroup, CommandItem, CommandState},
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    sidebar::{
        Sidebar, SidebarCollapsible, SidebarGroup, SidebarHeader, SidebarItem as _, SidebarMenu,
        SidebarMenuItem,
    },
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use ondes_audio::SourceSpec;

use crate::actions::{self, *};
use crate::compact::CompactView;
use crate::history::HistoryView;
use crate::info::{InfoOpened, InfoView};
use crate::library_view::{GroupKind, LibraryFolders, LibraryGroups, LibraryTracks};
use crate::now_playing::NowPlaying;
use crate::player::{Player, PlayerNotice};
use crate::player_bar::PlayerBar;
use crate::queue::Repeat;
use crate::radio::FavoritesModel;
use crate::radio_explore::RadioExplore;
use crate::radio_explore::toggle_favorite;
use crate::radio_favorites::RadioFavorites;
use crate::runtime::Rt;
use crate::settings::{Settings, WindowState};
use crate::system::System;
use crate::visualizer::VisualizerView;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// Écouter une URL ou ouvrir un fichier ; atteinte par le logo.
    Home,
    NowPlaying,
    History,
    /// Fiches artiste, album, titre.
    Info,
    RadioExplore,
    RadioFavorites,
    LibraryTracks,
    LibraryAlbums,
    LibraryArtists,
    LibraryFolders,
}

impl Page {
    /// Aide au développement (builds debug) : `ONDES_PAGE=albums` ouvre
    /// directement cette page, pour les captures d'écran automatisées.
    fn from_env() -> Option<Self> {
        if !cfg!(debug_assertions) {
            return None;
        }
        Some(Self::from_key(&std::env::var("ONDES_PAGE").ok()?))
    }

    /// Identifiant stable, enregistré dans l'état.
    fn key(self) -> &'static str {
        match self {
            Page::Home => "home",
            Page::NowPlaying => "now",
            Page::History => "history",
            Page::Info => "info",
            Page::RadioExplore => "explore",
            Page::RadioFavorites => "favorites",
            Page::LibraryTracks => "tracks",
            Page::LibraryAlbums => "albums",
            Page::LibraryArtists => "artists",
            Page::LibraryFolders => "folders",
        }
    }

    fn from_key(key: &str) -> Self {
        match key {
            "home" => Page::Home,
            "now" => Page::NowPlaying,
            "history" => Page::History,
            "info" => Page::Info,
            "favorites" => Page::RadioFavorites,
            "tracks" => Page::LibraryTracks,
            "albums" => Page::LibraryAlbums,
            "artists" => Page::LibraryArtists,
            "folders" => Page::LibraryFolders,
            _ => Page::RadioExplore,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Page::Home => "Accueil",
            Page::NowPlaying => "En cours",
            Page::History => "Historique",
            Page::Info => "Infos",
            Page::RadioExplore => "Explorer les radios",
            Page::RadioFavorites => "Radios favorites",
            Page::LibraryTracks => "Morceaux",
            Page::LibraryAlbums => "Albums",
            Page::LibraryArtists => "Artistes",
            Page::LibraryFolders => "Dossiers",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Page::Home => IconName::House,
            Page::NowPlaying => IconName::Disc3,
            Page::History => IconName::Clock,
            Page::Info => IconName::Info,
            Page::RadioExplore => IconName::Radio,
            Page::RadioFavorites => IconName::Heart,
            Page::LibraryTracks => IconName::ListMusic,
            Page::LibraryAlbums => IconName::DiscAlbum,
            Page::LibraryArtists => IconName::MicVocal,
            Page::LibraryFolders => IconName::FolderOpen,
        }
    }
}

/// Disposition de la fenêtre principale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// Barre latérale, pages et barre de lecture.
    Full,
    /// Lecteur réduit (voir [`CompactView`]).
    Compact,
    /// Visualiseur seul (voir [`VisualizerView`]).
    Immersive,
}

/// Position du logo dans la barre latérale (marges de gpui-kit : 12 + 8 px),
/// reprise par la ligne de titre des pages pour qu'ils soient alignés.
const TITLE_TOP: Pixels = px(20.);
const TITLE_HEIGHT: Pixels = px(32.);

/// En deçà, la fenêtre passe d'elle-même en vue compacte.
const COMPACT_BELOW: Size<Pixels> = Size {
    width: px(600.),
    height: px(420.),
};
/// Taille de la fenêtre en vue compacte, la première fois.
const COMPACT_SIZE: Size<Pixels> = Size {
    width: px(340.),
    height: px(440.),
};

pub struct Shell {
    page: Page,
    /// Tant que l'utilisateur n'a pas choisi, le thème suit le système.
    follow_system_theme: bool,
    /// Barre latérale réduite à ses icônes (Ctrl+B).
    sidebar_collapsed: bool,
    player: Entity<Player>,
    player_bar: Entity<PlayerBar>,
    now_playing: Entity<NowPlaying>,
    history: Entity<HistoryView>,
    info: Entity<InfoView>,
    explore: Entity<RadioExplore>,
    favorites: Entity<RadioFavorites>,
    tracks: Entity<LibraryTracks>,
    albums: Entity<LibraryGroups>,
    artists: Entity<LibraryGroups>,
    folders: Entity<LibraryFolders>,
    /// Champ « URL ou chemin » de la barre d'ouverture rapide.
    location: Entity<InputState>,
    palette: Entity<CommandState>,
    /// Disposition demandée (bouton, raccourci). La vue compacte s'affiche
    /// aussi quand la fenêtre est trop petite.
    layout: Layout,
    /// Disposition affichée à la dernière image.
    shown: Layout,
    /// Disposition à retrouver en quittant le mode immersif.
    before_immersive: Layout,
    /// Taille de la fenêtre avant la vue compacte, rendue en la quittant.
    full_size: Option<Size<Pixels>>,
    /// Vues créées à l'affichage et libérées ensuite (le visualiseur s'arrête).
    compact: Option<Entity<CompactView>>,
    immersive: Option<Entity<VisualizerView>>,
    _system: Entity<System>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(initial: Option<SourceSpec>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = Settings::get(cx);
        match saved.dark {
            Some(dark) => Theme::change(
                if dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                },
                Some(window),
                cx,
            ),
            None => Theme::sync_system_appearance(Some(window), cx),
        }
        let appearance_sub = cx.observe_window_appearance(window, |this, window, cx| {
            if this.follow_system_theme {
                Theme::sync_system_appearance(Some(window), cx);
            }
        });

        let location = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("URL d'une radio (mp3, aac, ogg, .pls, .m3u) ou chemin d'un fichier")
        });
        let enter_sub = cx.subscribe_in(&location, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.listen_location(cx);
            }
        });

        let player = cx.new(|cx| {
            let mut p = Player::new(saved.volume, &saved.audio, cx);
            p.queue.shuffle = saved.shuffle;
            p.queue.repeat = match saved.repeat.as_str() {
                "all" => Repeat::All,
                "one" => Repeat::One,
                _ => Repeat::Off,
            };
            // Proposée à la relecture (bouton Lecture), sans démarrer seule.
            p.resume = saved.last.clone().filter(|_| initial.is_none());
            p
        });
        let player_bar = cx.new(|cx| PlayerBar::new(player.clone(), window, cx));
        let now_playing = cx.new(|cx| NowPlaying::new(player.clone(), cx));
        let history = cx.new(|cx| HistoryView::new(player.clone(), window, cx));
        let info = cx.new(|cx| InfoView::new(player.clone(), cx));
        let explore = cx.new(|cx| RadioExplore::new(player.clone(), window, cx));
        let favorites = cx.new(|cx| RadioFavorites::new(player.clone(), window, cx));
        let tracks = cx.new(|cx| LibraryTracks::new(player.clone(), window, cx));
        let albums = cx.new(|cx| LibraryGroups::new(GroupKind::Albums, player.clone(), window, cx));
        let artists =
            cx.new(|cx| LibraryGroups::new(GroupKind::Artists, player.clone(), window, cx));
        let folders = cx.new(LibraryFolders::new);
        let palette = cx.new(|cx| CommandState::new(window, cx));
        let system = cx.new(|cx| System::new(player.clone(), window, cx));

        let subscriptions = vec![
            cx.subscribe(&info, |this, _, _: &InfoOpened, cx| {
                if this.page != Page::Info {
                    this.set_page(Page::Info, cx);
                }
            }),
            appearance_sub,
            enter_sub,
            // Mémorise volume, dernière source, aléatoire et répétition.
            cx.observe(&player, |_, player, cx| {
                let p = player.read(cx);
                let (volume, last, shuffle) = (p.volume(), p.last_source(), p.queue.shuffle);
                let repeat = match p.repeat() {
                    Repeat::Off => "off",
                    Repeat::All => "all",
                    Repeat::One => "one",
                };
                Settings::update(cx, |s| {
                    s.volume = volume;
                    if last.is_some() {
                        s.last = last;
                    }
                    s.shuffle = shuffle;
                    s.repeat = repeat.into();
                });
            }),
            cx.subscribe_in(
                &player,
                window,
                |_, _, notice: &PlayerNotice, window, cx| match notice {
                    PlayerNotice::Skipped { title, reason } => window.push_notification(
                        Notification::warning(format!("« {title} » ignoré : {reason}")),
                        cx,
                    ),
                    PlayerNotice::Recorded(done) => {
                        let size = format!("{:.1} Mo", done.bytes as f64 / 1e6);
                        let note = match &done.error {
                            Some(err) => Notification::error(format!(
                                "Enregistrement interrompu ({size}) : {err}"
                            )),
                            None => Notification::success(format!(
                                "Enregistré ({size}) : {}",
                                done.path.display()
                            )),
                        };
                        window.push_notification(note, cx);
                    }
                },
            ),
            cx.observe_window_bounds(window, |this, window, cx| {
                if window.is_fullscreen() {
                    return;
                }
                let maximized = window.is_maximized();
                let b = window.bounds();
                // En vue compacte demandée, c'est la taille de cette vue qu'on retient.
                if this.layout == Layout::Compact {
                    if !maximized {
                        let state = WindowState {
                            x: f32::from(b.origin.x),
                            y: f32::from(b.origin.y),
                            width: f32::from(b.size.width),
                            height: f32::from(b.size.height),
                            maximized: false,
                        };
                        Settings::update(cx, |s| s.compact.window = Some(state));
                    }
                    return;
                }
                Settings::update(cx, |s| {
                    // Taille « normale » gardée quand la fenêtre est agrandie.
                    if maximized {
                        if let Some(w) = &mut s.window {
                            w.maximized = true;
                        }
                    } else {
                        s.window = Some(WindowState {
                            x: f32::from(b.origin.x),
                            y: f32::from(b.origin.y),
                            width: f32::from(b.size.width),
                            height: f32::from(b.size.height),
                            maximized: false,
                        });
                    }
                });
            }),
        ];

        let shell = Self {
            page: Page::from_env().unwrap_or(Page::from_key(&saved.page)),
            follow_system_theme: saved.dark.is_none(),
            sidebar_collapsed: saved.sidebar_collapsed,
            player,
            player_bar,
            now_playing,
            history,
            info,
            explore,
            favorites,
            tracks,
            albums,
            artists,
            folders,
            location,
            palette,
            layout: Layout::Full,
            shown: Layout::Full,
            before_immersive: Layout::Full,
            full_size: None,
            compact: None,
            immersive: None,
            _system: system,
            _subscriptions: subscriptions,
        };
        shell.register_actions(cx.entity().downgrade(), cx);
        // Aide au développement (builds debug) : palette ouverte au démarrage.
        if cfg!(debug_assertions)
            && let Some(what) = std::env::var_os("ONDES_PALETTE")
        {
            cx.defer_in(window, move |this, window, cx| {
                if what == "about" {
                    crate::about::open(window, cx);
                } else if what == "audio" {
                    crate::audio_settings::open(this.player.clone(), window, cx);
                } else if what == "compact" {
                    this.toggle_compact(window, cx);
                } else if what == "visual" {
                    this.toggle_visualizer(window, cx);
                } else {
                    this.open_palette(window, cx);
                }
            });
        }
        // Aide au développement (builds debug) : fiche ouverte au démarrage,
        // `ONDES_INFO="artist:Nom"`, `"album:Artiste - Album"` ou `"Artiste - Titre"`.
        if cfg!(debug_assertions)
            && let Ok(what) = std::env::var("ONDES_INFO")
        {
            let target = if let Some(name) = what.strip_prefix("artist:") {
                Some(crate::info::Target::Artist {
                    id: None,
                    name: name.into(),
                })
            } else if let Some(rest) = what.strip_prefix("album:") {
                rest.split_once(" - ")
                    .map(|(artist, title)| crate::info::Target::Album {
                        id: None,
                        artist: artist.into(),
                        title: title.into(),
                    })
            } else {
                crate::info::Target::for_announced(&what)
            };
            if let Some(target) = target {
                cx.defer(move |cx| crate::info::show(target, cx));
            }
        }
        shell.check_runtime(window, cx);
        crate::update::check_on_startup(window, cx);
        if let Some(spec) = initial {
            shell.play(spec, cx);
        }
        shell
    }

    /// Aller-retour GPUI → tokio → GPUI : prouve que le pont fonctionne.
    /// Vérifie au démarrage que le runtime tokio (réseau) répond : résultat
    /// dans le journal, notification seulement en cas d'échec.
    fn check_runtime(&self, window: &mut Window, cx: &mut Context<Self>) {
        let task = Rt::spawn(cx, ondes_core::rt::probe());
        cx.spawn_in(window, async move |this, cx| match task.await {
            Ok(elapsed) => tracing::info!(?elapsed, "runtime tokio prêt"),
            Err(err) => {
                tracing::error!("runtime tokio indisponible : {err}");
                this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        Notification::error(format!(
                            "Le réseau ne répond pas (radios, annuaire, mises à jour) : {err}"
                        )),
                        cx,
                    );
                })
                .ok();
            }
        })
        .detach();
    }

    fn play(&self, spec: SourceSpec, cx: &mut Context<Self>) {
        self.player.update(cx, |p, cx| p.load(spec, cx));
    }

    fn listen_location(&mut self, cx: &mut Context<Self>) {
        let text = self.location.read(cx).value();
        if let Some(spec) = SourceSpec::parse(&text) {
            self.play(spec, cx);
        }
    }

    fn open_file_dialog(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Écouter".into()),
        });
        cx.spawn(async move |this, cx| {
            let path = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Err(e)) => {
                    tracing::warn!("boîte de dialogue indisponible : {e:#}");
                    None
                }
                _ => None,
            };
            if let Some(path) = path {
                this.update(cx, |this, cx| this.play(SourceSpec::File(path), cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Accueil : écouter une URL (radio, playlist) ou ouvrir un fichier.
    fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_6()
            .child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_2xl()
                            .font_semibold()
                            .child("Que voulez-vous écouter ?"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("Collez l'adresse d'une radio ou d'une playlist, ou ouvrez un fichier audio."),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .max_w(px(720.))
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.location).cleanable(true)),
                    )
                    .child(
                        Button::new("listen")
                            .icon(IconName::Play)
                            .label("Écouter")
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.listen_location(cx))),
                    )
                    .child(
                        Button::new("open-file")
                            .icon(IconName::FolderOpen)
                            .label("Ouvrir un fichier…")
                            .outline()
                            .on_click(cx.listener(|this, _, _, cx| this.open_file_dialog(cx))),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Vous pouvez aussi glisser-déposer un fichier dans la fenêtre."),
            )
    }

    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.follow_system_theme = false;
        let dark = !cx.theme().is_dark();
        Theme::change(
            if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            None,
            cx,
        );
        Settings::update(cx, |s| s.dark = Some(dark));
    }

    /// Plie la barre latérale (icônes seules) ou la déplie.
    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        let collapsed = self.sidebar_collapsed;
        Settings::update(cx, |s| s.sidebar_collapsed = collapsed);
        cx.notify();
    }

    /// Bascule entre la vue complète et la vue compacte ; la fenêtre prend
    /// la taille mémorisée de chacune.
    fn toggle_compact(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layout == Layout::Immersive {
            self.leave_immersive(window);
        }
        let compact = self.layout == Layout::Compact || too_small(window);
        if compact {
            self.layout = Layout::Full;
            let saved = Settings::get(cx).window;
            let size = self
                .full_size
                .take()
                .or_else(|| saved.map(|w| size(px(w.width), px(w.height))))
                .filter(|s| !fits_compact(*s))
                .unwrap_or(size(px(1100.), px(720.)));
            window.resize(size);
        } else {
            if window.is_fullscreen() {
                window.toggle_fullscreen();
            }
            if window.is_maximized() {
                window.zoom_window();
            }
            self.full_size = Some(window.viewport_size());
            self.layout = Layout::Compact;
            let size = Settings::get(cx)
                .compact
                .window
                .map(|w| size(px(w.width), px(w.height)))
                .unwrap_or(COMPACT_SIZE);
            window.resize(size);
        }
        cx.notify();
    }

    /// Entre dans le mode immersif (visualiseur seul) ou en sort.
    fn toggle_visualizer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layout == Layout::Immersive {
            self.leave_immersive(window);
        } else {
            self.before_immersive = self.layout;
            self.layout = Layout::Immersive;
        }
        cx.notify();
    }

    fn leave_immersive(&mut self, window: &mut Window) {
        if window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        self.layout = self.before_immersive;
    }

    fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        // Page « Infos » encore vide : fiche de ce qui joue.
        if page == Page::Info && self.info.read(cx).is_empty() {
            self.info.update(cx, |i, cx| i.open_now_playing(cx));
        }
        if page != Page::NowPlaying {
            self.now_playing.update(cx, |n, cx| n.suspend(cx));
        }
        self.page = page;
        Settings::update(cx, |s| s.page = page.key().into());
        cx.notify();
    }

    /// Actions globales : raccourcis clavier, palette, menus.
    ///
    /// Enregistrées au niveau de l'application (et non de la vue) pour
    /// fonctionner aussi depuis la palette, ouverte dans une couche à part.
    /// Celles qui ont besoin de la fenêtre sont différées après l'action en
    /// cours, quand la fenêtre est de nouveau disponible.
    fn register_actions(&self, shell: WeakEntity<Self>, cx: &mut App) {
        let player = self.player.clone();

        macro_rules! on_player {
            ($action:ty, |$p:ident, $cx:ident| $body:expr) => {{
                let player = player.clone();
                cx.on_action(move |_: &$action, cx: &mut App| {
                    player.update(cx, |$p, $cx| $body);
                });
            }};
        }
        on_player!(TogglePlay, |p, cx| p.toggle_play(cx));
        on_player!(actions::Stop, |p, cx| p.stop(cx));
        on_player!(Next, |p, cx| p.next(cx));
        on_player!(Previous, |p, cx| p.prev(cx));
        on_player!(SeekForward, |p, cx| p.seek_by(10_000, cx));
        on_player!(SeekBackward, |p, cx| p.seek_by(-10_000, cx));
        on_player!(VolumeUp, |p, cx| p.change_volume(0.05, cx));
        on_player!(VolumeDown, |p, cx| p.change_volume(-0.05, cx));
        on_player!(ToggleShuffle, |p, cx| p.toggle_shuffle(cx));
        on_player!(CycleRepeat, |p, cx| p.cycle_repeat(cx));
        on_player!(ToggleRecording, |p, cx| p.toggle_recording(cx));

        macro_rules! with_window {
            ($action:ty, |$this:ident, $window:ident, $cx:ident| $body:expr) => {{
                let shell = shell.clone();
                cx.on_action(move |_: &$action, cx: &mut App| {
                    let shell = shell.clone();
                    cx.defer(move |cx| {
                        let Some(handle) = cx.active_window() else {
                            return;
                        };
                        handle
                            .update(cx, |_, $window, cx| {
                                shell.update(cx, |$this, $cx| $body).ok();
                            })
                            .ok();
                    });
                });
            }};
        }
        with_window!(OpenPalette, |this, window, cx| this
            .open_palette(window, cx));
        with_window!(ToggleCompact, |this, window, cx| this
            .toggle_compact(window, cx));
        with_window!(ToggleVisualizer, |this, window, cx| this
            .toggle_visualizer(window, cx));
        with_window!(About, |_this, window, cx| crate::about::open(window, cx));
        with_window!(OpenAudioSettings, |this, window, cx| {
            crate::audio_settings::open(this.player.clone(), window, cx)
        });
        with_window!(FocusLocation, |this, window, cx| {
            this.set_page(Page::Home, cx);
            this.location.update(cx, |s, cx| s.focus(window, cx));
        });
        with_window!(ToggleFavorite, |this, window, cx| {
            let station = this.player.read(cx).current_station();
            match station {
                Some(station) => toggle_favorite(&FavoritesModel::global(cx), &station, window, cx),
                None => window.push_notification(
                    Notification::info("Aucune radio en cours à ajouter aux favoris"),
                    cx,
                ),
            }
        });
        with_window!(ToggleHistory, |_this, window, cx| {
            let enabled = !Settings::get(cx).history;
            Settings::update(cx, |s| s.history = enabled);
            window.push_notification(
                Notification::info(if enabled {
                    "Historique d'écoute activé"
                } else {
                    "Historique d'écoute en pause"
                }),
                cx,
            );
        });
        with_window!(CheckUpdates, |_this, window, cx| crate::update::check(
            true, window, cx
        ));
        with_window!(ToggleUpdateCheck, |_this, window, cx| {
            let enabled = !Settings::get(cx).check_updates;
            Settings::update(cx, |s| s.check_updates = enabled);
            window.push_notification(
                Notification::info(if enabled {
                    "Vérification des mises à jour activée"
                } else {
                    "Vérification des mises à jour désactivée"
                }),
                cx,
            );
        });
        with_window!(ToggleNotifications, |_this, window, cx| {
            let enabled = !Settings::get(cx).notifications;
            Settings::update(cx, |s| s.notifications = enabled);
            window.push_notification(
                Notification::info(if enabled {
                    "Notifications activées"
                } else {
                    "Notifications désactivées"
                }),
                cx,
            );
        });

        macro_rules! on_shell {
            ($action:ty, |$this:ident, $cx:ident| $body:expr) => {{
                let shell = shell.clone();
                cx.on_action(move |_: &$action, cx: &mut App| {
                    shell.update(cx, |$this, $cx| $body).ok();
                });
            }};
        }
        on_shell!(OpenFile, |this, cx| this.open_file_dialog(cx));

        on_shell!(ToggleTheme, |this, cx| this.toggle_theme(cx));
        on_shell!(ToggleSidebar, |this, cx| this.toggle_sidebar(cx));
        on_shell!(GoHome, |this, cx| this.set_page(Page::Home, cx));
        on_shell!(GoNowPlaying, |this, cx| this.set_page(Page::NowPlaying, cx));
        on_shell!(GoHistory, |this, cx| this.set_page(Page::History, cx));
        with_window!(ShowInfo, |this, window, cx| {
            let found = this.info.update(cx, |i, cx| i.open_now_playing(cx));
            if !found {
                window.push_notification(
                    Notification::info(
                        "Rien à chercher : il faut un morceau avec son artiste, \
                         ou une radio qui annonce « Artiste - Titre »",
                    ),
                    cx,
                );
            }
        });
        on_shell!(GoExplore, |this, cx| this.set_page(Page::RadioExplore, cx));
        on_shell!(GoFavorites, |this, cx| this
            .set_page(Page::RadioFavorites, cx));
        on_shell!(GoTracks, |this, cx| this.set_page(Page::LibraryTracks, cx));
        on_shell!(GoAlbums, |this, cx| this.set_page(Page::LibraryAlbums, cx));
        on_shell!(GoArtists, |this, cx| this
            .set_page(Page::LibraryArtists, cx));
        on_shell!(GoFolders, |this, cx| this
            .set_page(Page::LibraryFolders, cx));
    }

    /// Palette de commandes (Ctrl+K) : toutes les actions, avec leur raccourci.
    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.palette.clone();
        state.update(cx, |s, cx| s.set_query("", window, cx));
        let focus_on_mount = Rc::new(Cell::new(true));
        window.open_dialog(cx, move |dialog, _, _| {
            let state = state.clone();
            let focus_on_mount = focus_on_mount.clone();
            dialog
                .close_button(false)
                .p_0()
                .content(move |content, window, cx| {
                    if focus_on_mount.replace(false) {
                        let state = state.clone();
                        window.defer(cx, move |window, cx| {
                            state.read(cx).focus_handle(cx).focus(window, cx);
                        });
                    }
                    let mut command = Command::new(&state)
                        .bordered(false)
                        .rounded(cx.theme().radius_lg)
                        .max_h(px(420.))
                        .placeholder("Rechercher une commande…")
                        .on_confirm(|_, window, cx| window.close_dialog(cx));
                    for (label, entries) in actions::palette() {
                        command = command.group(CommandGroup::new().label(label).items(
                            entries.into_iter().map(|e| {
                                CommandItem::new()
                                    .label(e.label)
                                    .icon(e.icon)
                                    .keywords(e.keywords.iter().copied())
                                    .action((e.action)())
                            }),
                        ));
                    }
                    content.child(command)
                })
        });
    }

    fn nav_item(&self, page: Page, cx: &mut Context<Self>) -> SidebarMenuItem {
        let entity = cx.entity().downgrade();
        SidebarMenuItem::new(page.title())
            .icon(page.icon())
            .active(self.page == page)
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.set_page(page, cx)).ok();
            })
    }

    fn render_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let collapsed = self.sidebar_collapsed;

        let logo = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size_8()
            .rounded(theme.radius)
            .bg(theme.sidebar_primary)
            .text_color(theme.sidebar_primary_foreground)
            .child(IconName::RadioTower);
        // Le logo mène à l'accueil (écouter une URL, ouvrir un fichier).
        let header = SidebarHeader::new()
            .selected(self.page == Page::Home)
            // Pliée, il ne reste que 32 px : pas de marge interne, et une marge
            // haute qui garde le logo à la même hauteur (20 px) que dépliée.
            .when(collapsed, |this| this.p_0().mt_3().justify_center())
            .child(
                h_flex()
                    .id("home-link")
                    .flex_1()
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .tooltip(|window, cx| {
                        Tooltip::new("Accueil")
                            .action(&FocusLocation, None)
                            .build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.set_page(Page::Home, cx)))
                    .child(logo)
                    .when(!collapsed, |this| {
                        this.child(div().font_semibold().child("Ondes"))
                    }),
            );

        // Réglages, « À propos » et thème : une liste comme la navigation,
        // réduite aux icônes (avec infobulle) quand la barre est pliée.
        let footer = v_flex()
            .w_full()
            .gap_2()
            .when(collapsed, |this| this.items_center())
            .child(self.footer_menu(cx).w_full().collapsed(collapsed).render(
                "footer-menu",
                window,
                cx,
            ));

        Sidebar::new("main-sidebar")
            .w(px(232.))
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(collapsed)
            .header(header)
            .child(
                SidebarGroup::new("Écoute").child(SidebarMenu::new().children([
                    self.nav_item(Page::NowPlaying, cx),
                    self.nav_item(Page::History, cx),
                    self.nav_item(Page::Info, cx),
                ])),
            )
            .child(
                SidebarGroup::new("Radios").child(SidebarMenu::new().children([
                    self.nav_item(Page::RadioExplore, cx),
                    self.nav_item(Page::RadioFavorites, cx),
                ])),
            )
            .child(
                SidebarGroup::new("Bibliothèque").child(SidebarMenu::new().children([
                    self.nav_item(Page::LibraryTracks, cx),
                    self.nav_item(Page::LibraryAlbums, cx),
                    self.nav_item(Page::LibraryArtists, cx),
                    self.nav_item(Page::LibraryFolders, cx),
                ])),
            )
            .footer(footer)
    }

    /// Réglages audio, « À propos » et thème, en bas de la barre latérale.
    fn footer_menu(&self, cx: &mut Context<Self>) -> SidebarMenu {
        let is_dark = cx.theme().is_dark();
        let player = self.player.clone();
        let shell = cx.entity().downgrade();
        SidebarMenu::new().children([
            SidebarMenuItem::new("Réglages audio")
                .icon(IconName::SlidersHorizontal)
                .on_click(move |_, window, cx| {
                    crate::audio_settings::open(player.clone(), window, cx)
                }),
            SidebarMenuItem::new("À propos")
                .icon(IconName::Info)
                .on_click(|_, window, cx| crate::about::open(window, cx)),
            SidebarMenuItem::new(if is_dark {
                "Thème clair"
            } else {
                "Thème sombre"
            })
            .icon(if is_dark {
                IconName::Sun
            } else {
                IconName::Moon
            })
            .on_click(move |_, _, cx| {
                shell.update(cx, |this, cx| this.toggle_theme(cx)).ok();
            }),
        ])
    }

    fn render_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.sidebar_collapsed;
        // Ligne de titre à la hauteur du logo de la barre latérale (20 à 52 px).
        let title = h_flex()
            .h(TITLE_HEIGHT)
            .flex_shrink_0()
            .gap_2()
            .items_center()
            .child(
                Button::new("toggle-sidebar")
                    .icon(if collapsed {
                        IconName::PanelLeftOpen
                    } else {
                        IconName::PanelLeftClose
                    })
                    .ghost()
                    .small()
                    .tooltip_with_action(
                        if collapsed {
                            "Déplier la barre latérale"
                        } else {
                            "Plier la barre latérale"
                        },
                        &ToggleSidebar,
                        None,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(div().text_xl().font_semibold().child(self.page.title()));
        v_flex()
            .flex_1()
            .h_full()
            .min_h_0()
            .px_6()
            .pb_6()
            .pt(TITLE_TOP)
            .gap_4()
            .child(title)
            // Pages en cache : redessinées seulement quand elles changent, et non
            // à chaque image du spectre de la barre de lecture.
            .child({
                let style = || StyleRefinement::default().size_full().min_h_0();
                match self.page {
                    Page::Home => self.render_home(cx).into_any_element(),
                    Page::NowPlaying => self.now_playing.clone().cached(style()).into_any_element(),
                    Page::History => self.history.clone().cached(style()).into_any_element(),
                    Page::Info => self.info.clone().cached(style()).into_any_element(),
                    Page::RadioExplore => self.explore.clone().cached(style()).into_any_element(),
                    Page::RadioFavorites => {
                        self.favorites.clone().cached(style()).into_any_element()
                    }
                    Page::LibraryTracks => self.tracks.clone().cached(style()).into_any_element(),
                    Page::LibraryAlbums => self.albums.clone().cached(style()).into_any_element(),
                    Page::LibraryArtists => self.artists.clone().cached(style()).into_any_element(),
                    Page::LibraryFolders => self.folders.clone().cached(style()).into_any_element(),
                }
            })
    }
}

/// Fenêtre trop petite pour la vue complète.
fn too_small(window: &Window) -> bool {
    fits_compact(window.viewport_size())
}

fn fits_compact(size: Size<Pixels>) -> bool {
    size.width < COMPACT_BELOW.width || size.height < COMPACT_BELOW.height
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = match self.layout {
            Layout::Full if too_small(window) => Layout::Compact,
            layout => layout,
        };
        if shown != self.shown {
            // Page « En cours » masquée : son visualiseur s'arrête.
            if self.shown == Layout::Full {
                self.now_playing.update(cx, |n, cx| n.suspend(cx));
            }
            if shown != Layout::Compact {
                self.compact = None;
            }
            if shown != Layout::Immersive {
                self.immersive = None;
            }
            self.shown = shown;
        }

        let content = match shown {
            Layout::Full => v_flex()
                .size_full()
                .child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .child(self.render_sidebar(window, cx))
                        .child(self.render_content(cx)),
                )
                .child(self.player_bar.clone())
                .into_any_element(),
            Layout::Compact => {
                let player = self.player.clone();
                self.compact
                    .get_or_insert_with(|| cx.new(|cx| CompactView::new(player, window, cx)))
                    .clone()
                    .into_any_element()
            }
            Layout::Immersive => {
                let player = self.player.clone();
                self.immersive
                    .get_or_insert_with(|| cx.new(|cx| VisualizerView::new(player, window, cx)))
                    .clone()
                    .into_any_element()
            }
        };

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            // Glisser-déposer un fichier audio n'importe où dans la fenêtre.
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if let Some(path) = paths.paths().first() {
                    this.play(SourceSpec::File(path.clone()), cx);
                }
            }))
            .text_color(cx.theme().foreground)
            .child(content)
    }
}
