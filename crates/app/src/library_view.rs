//! Vues de la bibliothèque : Morceaux, Albums, Artistes, Dossiers.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, StyledExt, WindowExt as _,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    progress::Progress,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ondes_library::search::{Query, fold};
use ondes_library::{ScanProgress, Track};

use crate::info::Target;
use crate::library::LibraryModel;
use crate::player::Player;
use crate::player_bar::clock;

const TRACK_ROW: f32 = 36.;

/// Colonne proportionnelle : `weight` parts de la largeur restante.
trait Grow: Styled + Sized {
    fn grow(mut self, weight: f32) -> Self {
        let style = self.style();
        style.flex_grow = Some(weight);
        style.flex_shrink = Some(1.);
        style.flex_basis = Some(px(0.).into());
        self
    }
}

impl<E: Styled> Grow for E {}
const GROUP_ROW: f32 = 64.;

// ─── Ajout de dossier (partagé) ────────────────────────────────────────────

fn add_folder(path: PathBuf, window: &mut Window, cx: &mut App) {
    let library = LibraryModel::global(cx);
    if let Err(e) = library.update(cx, |l, cx| l.add_folder(&path, cx)) {
        window.push_notification(Notification::error(e), cx);
    }
}

fn pick_folder(window: &mut Window, cx: &mut App) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: true,
        prompt: Some("Ajouter à la bibliothèque".into()),
    });
    window
        .spawn(cx, async move |cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                cx.update(|window, cx| {
                    for path in paths {
                        add_folder(path, window, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
}

/// Écran d'accueil d'une bibliothèque vide.
fn empty_library(cx: &App) -> AnyElement {
    let theme = cx.theme();
    let music = crate::paths::music_dir();
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .text_color(theme.muted_foreground)
        .child(Icon::new(IconName::LibraryBig).large())
        .child(div().text_sm().child("Ta bibliothèque est vide."))
        .child(
            h_flex()
                .gap_2()
                .when_some(music, |row, music| {
                    let label = format!("Ajouter {}", music.display());
                    row.child(
                        Button::new("add-music-dir")
                            .icon(IconName::FolderPlus)
                            .label(label)
                            .primary()
                            .on_click(move |_, window, cx| add_folder(music.clone(), window, cx)),
                    )
                })
                .child(
                    Button::new("pick-folder")
                        .icon(IconName::FolderOpen)
                        .label("Choisir un dossier…")
                        .outline()
                        .on_click(|_, window, cx| pick_folder(window, cx)),
                ),
        )
        .into_any_element()
}

fn search_input(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

fn search_field(state: &Entity<InputState>) -> impl IntoElement {
    Input::new(state)
        .prefix(Icon::new(IconName::Search).small())
        .cleanable(true)
}

// ─── Table de morceaux ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    /// Ordre de la portée (album : disque / piste ; tout : artiste, album, piste).
    Natural,
    Title,
    Artist,
    Album,
    Duration,
}

/// Morceaux affichés : toute la bibliothèque, ou une sélection (album, artiste…).
#[derive(Clone)]
pub enum Scope {
    All,
    Some(Arc<Vec<usize>>),
}

pub struct TrackTable {
    player: Entity<Player>,
    library: Entity<LibraryModel>,
    scope: Scope,
    search: Option<Entity<InputState>>,
    query: Query,
    sort: SortKey,
    descending: bool,
    /// Indices (dans `data.tracks`) des lignes affichées, dans l'ordre.
    rows: Vec<usize>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl TrackTable {
    pub fn new(
        player: Entity<Player>,
        scope: Scope,
        with_search: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let library = LibraryModel::global(cx);
        let search = with_search
            .then(|| search_input("Rechercher un titre, un artiste, un album…", window, cx));
        let mut subscriptions = vec![
            cx.observe(&library, |this, _, cx| this.recompute(cx)),
            cx.observe(&player, |_, _, cx| cx.notify()),
        ];
        if let Some(search) = &search {
            subscriptions.push(cx.subscribe(search, |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.query = Query::new(&input.read(cx).value());
                    this.recompute(cx);
                }
            }));
        }
        let mut this = Self {
            player,
            library,
            scope,
            search,
            query: Query::default(),
            sort: SortKey::Natural,
            descending: false,
            rows: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.recompute(cx);
        this
    }

    fn recompute(&mut self, cx: &mut Context<Self>) {
        let data = self.library.read(cx).data.clone();
        let tracks = &data.tracks;
        let base: Vec<usize> = match &self.scope {
            Scope::All => {
                // Ordre naturel : artiste, album, disque, piste.
                let mut all: Vec<usize> = (0..tracks.len()).collect();
                all.sort_by_cached_key(|&i| {
                    let t = &tracks[i];
                    (
                        fold(t.album_artist_or_artist().unwrap_or("\u{10FFFF}")),
                        fold(t.album.as_deref().unwrap_or_default()),
                        t.disc_no.unwrap_or(1),
                        t.track_no.unwrap_or(u32::MAX),
                        fold(&t.title),
                    )
                });
                all
            }
            Scope::Some(ixs) => ixs.iter().copied().filter(|&i| i < tracks.len()).collect(),
        };
        let mut rows = data.filter(base, &self.query);
        match self.sort {
            SortKey::Natural => {}
            SortKey::Title => rows.sort_by_cached_key(|&i| fold(&tracks[i].title)),
            SortKey::Artist => rows
                .sort_by_cached_key(|&i| fold(tracks[i].artist.as_deref().unwrap_or("\u{10FFFF}"))),
            SortKey::Album => rows.sort_by_cached_key(|&i| {
                (
                    fold(tracks[i].album.as_deref().unwrap_or("\u{10FFFF}")),
                    tracks[i].disc_no.unwrap_or(1),
                    tracks[i].track_no.unwrap_or(u32::MAX),
                )
            }),
            SortKey::Duration => rows.sort_by_key(|&i| tracks[i].duration),
        }
        if self.descending {
            rows.reverse();
        }
        self.rows = rows;
        cx.notify();
    }

    fn set_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        if self.sort == key {
            // Deuxième clic : sens inverse ; troisième : ordre naturel.
            if self.descending {
                self.sort = SortKey::Natural;
                self.descending = false;
            } else {
                self.descending = true;
            }
        } else {
            self.sort = key;
            self.descending = false;
        }
        self.recompute(cx);
    }

    /// Joue les lignes affichées à partir de `row` (`shuffle` : ordre aléatoire).
    pub fn play_from(&mut self, row: usize, shuffle: bool, cx: &mut Context<Self>) {
        let data = self.library.read(cx).data.clone();
        let tracks: Vec<Track> = self.rows.iter().map(|&i| data.tracks[i].clone()).collect();
        if tracks.is_empty() {
            return;
        }
        let start = if shuffle {
            fastrand::usize(..tracks.len())
        } else {
            row
        };
        self.player.update(cx, |p, cx| {
            p.queue.set_shuffle(shuffle);
            p.play_tracks(tracks, start, cx);
        });
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let arrow = |key: SortKey| -> &'static str {
            match (self.sort == key, self.descending) {
                (true, false) => " ↑",
                (true, true) => " ↓",
                _ => "",
            }
        };
        let column =
            |id: &'static str, label: &'static str, key: SortKey, cx: &mut Context<Self>| {
                div()
                    .id(id)
                    .cursor_pointer()
                    .truncate()
                    .hover(|d| d.text_color(cx.theme().foreground))
                    .child(format!("{label}{}", arrow(key)))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_sort(key, cx)))
            };
        h_flex()
            .h(px(28.))
            .px_3()
            .gap_3()
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .border_b_1()
            .border_color(theme.border)
            .child(div().w(px(28.)).text_right().child("#"))
            .child(
                column("sort-title", "Titre", SortKey::Title, cx)
                    .grow(3.)
                    .min_w_0(),
            )
            .child(
                column("sort-artist", "Artiste", SortKey::Artist, cx)
                    .grow(2.)
                    .min_w_0(),
            )
            .child(
                column("sort-album", "Album", SortKey::Album, cx)
                    .grow(2.)
                    .min_w_0(),
            )
            .child(
                column("sort-duration", "Durée", SortKey::Duration, cx)
                    .w(px(56.))
                    .text_right(),
            )
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let data = self.library.read(cx).data.clone();
        let current = self.player.read(cx).current_track().map(|t| t.path.clone());
        let theme = cx.theme().clone();
        range
            .filter_map(|row| {
                let track = data.tracks.get(*self.rows.get(row)?)?;
                let is_current = current.as_ref() == Some(&track.path);
                let muted = theme.muted_foreground;
                Some(
                    h_flex()
                        .id(ElementId::NamedInteger("track".into(), row as u64))
                        .h(px(TRACK_ROW))
                        .w_full()
                        .px_3()
                        .gap_3()
                        .items_center()
                        .text_sm()
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .hover(|d| d.bg(theme.accent))
                        .when(is_current, |d| d.bg(theme.accent).text_color(theme.primary))
                        .on_click(cx.listener(move |this, _, _, cx| this.play_from(row, false, cx)))
                        .child(
                            div()
                                .w(px(28.))
                                .text_right()
                                .text_xs()
                                .text_color(muted)
                                .child(match (is_current, track.track_no) {
                                    (true, _) => {
                                        Icon::new(IconName::AudioLines).xsmall().into_any_element()
                                    }
                                    (false, Some(n)) => n.to_string().into_any_element(),
                                    (false, None) => "".into_any_element(),
                                }),
                        )
                        .child(
                            div()
                                .grow(3.)
                                .min_w_0()
                                .truncate()
                                .child(track.title.clone()),
                        )
                        .child(
                            div()
                                .grow(2.)
                                .min_w_0()
                                .truncate()
                                .text_color(muted)
                                .child(track.artist.clone().unwrap_or_default()),
                        )
                        .child(
                            div()
                                .grow(2.)
                                .min_w_0()
                                .truncate()
                                .text_color(muted)
                                .child(track.album.clone().unwrap_or_default()),
                        )
                        .child(
                            div()
                                .w(px(56.))
                                .text_right()
                                .text_xs()
                                .text_color(muted)
                                .child(clock(track.duration)),
                        ),
                )
            })
            .collect()
    }
}

impl Render for TrackTable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.rows.len();
        let total_secs: u64 = {
            let data = self.library.read(cx).data.clone();
            self.rows
                .iter()
                .map(|&i| data.tracks[i].duration.as_secs())
                .sum()
        };
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .when_some(self.search.clone(), |col, search| {
                col.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().flex_1().child(search_field(&search)))
                        .child(
                            Button::new("play-all")
                                .icon(IconName::Play)
                                .label("Tout lire")
                                .outline()
                                .disabled(count == 0)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.play_from(0, false, cx)),
                                ),
                        )
                        .child(
                            Button::new("shuffle-all")
                                .icon(IconName::Shuffle)
                                .label("Aléatoire")
                                .outline()
                                .disabled(count == 0)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.play_from(0, true, cx)),
                                ),
                        ),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{count} morceau{} · {}",
                        if count > 1 { "x" } else { "" },
                        clock(std::time::Duration::from_secs(total_secs))
                    )),
            )
            .child(self.header(cx))
            .child(
                uniform_list(
                    "tracks",
                    count,
                    cx.processor(|this, range: Range<usize>, _, cx| this.render_rows(range, cx)),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .min_h_0(),
            )
    }
}

// ─── Page Morceaux ─────────────────────────────────────────────────────────

pub struct LibraryTracks {
    table: Entity<TrackTable>,
    library: Entity<LibraryModel>,
    _subscriptions: Vec<Subscription>,
}

impl LibraryTracks {
    pub fn new(player: Entity<Player>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let library = LibraryModel::global(cx);
        let table = cx.new(|cx| TrackTable::new(player, Scope::All, true, window, cx));
        Self {
            _subscriptions: vec![cx.observe(&library, |_, _, cx| cx.notify())],
            table,
            library,
        }
    }
}

impl Render for LibraryTracks {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.library.read(cx).folders.is_empty() {
            return empty_library(cx);
        }
        self.table.clone().into_any_element()
    }
}

// ─── Pages Albums et Artistes (liste + détail) ─────────────────────────────

/// Groupe affiché dans une liste (album ou artiste).
struct Group {
    title: String,
    subtitle: String,
    cover: Option<PathBuf>,
    tracks: Arc<Vec<usize>>,
    /// Fiche de l'album ou de l'artiste (page « Infos »).
    info: Option<Target>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    Albums,
    Artists,
}

/// Groupe ouvert : titre, sous-titre, pochette, table de ses morceaux, fiche.
type Detail = (
    String,
    String,
    Option<PathBuf>,
    Entity<TrackTable>,
    Option<Target>,
);

pub struct LibraryGroups {
    kind: GroupKind,
    player: Entity<Player>,
    library: Entity<LibraryModel>,
    search: Entity<InputState>,
    groups: Vec<Group>,
    /// Groupe ouvert : titre, sous-titre, pochette, table de ses morceaux, fiche.
    detail: Option<Detail>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl LibraryGroups {
    pub fn new(
        kind: GroupKind,
        player: Entity<Player>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let library = LibraryModel::global(cx);
        let search = search_input(
            match kind {
                GroupKind::Albums => "Rechercher un album ou un artiste…",
                GroupKind::Artists => "Rechercher un artiste…",
            },
            window,
            cx,
        );
        let subscriptions = vec![
            cx.observe(&library, |this, _, cx| this.recompute(cx)),
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.recompute(cx);
                }
            }),
        ];
        let mut this = Self {
            kind,
            player,
            library,
            search,
            groups: Vec::new(),
            detail: None,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.recompute(cx);
        this
    }

    fn recompute(&mut self, cx: &mut Context<Self>) {
        let data = self.library.read(cx).data.clone();
        let query = Query::new(&self.search.read(cx).value());
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n > 1 { many } else { one });
        self.groups = match self.kind {
            GroupKind::Albums => data
                .albums
                .iter()
                .filter(|a| {
                    query.matches(&fold(&format!(
                        "{} {}",
                        a.title,
                        a.artist.as_deref().unwrap_or_default()
                    )))
                })
                .map(|a| {
                    let mut parts =
                        vec![a.artist.clone().unwrap_or_else(|| "Artiste inconnu".into())];
                    if let Some(year) = a.year {
                        parts.push(year.to_string());
                    }
                    parts.push(plural(a.tracks.len(), "morceau", "morceaux"));
                    parts.push(clock(a.duration));
                    Group {
                        title: a.title.clone(),
                        subtitle: parts.join(" · "),
                        cover: a.cover.clone(),
                        tracks: Arc::new(a.tracks.clone()),
                        info: a.artist.clone().map(|artist| Target::Album {
                            id: None,
                            artist,
                            title: a.title.clone(),
                        }),
                    }
                })
                .collect(),
            GroupKind::Artists => data
                .artists
                .iter()
                .filter(|a| query.matches(&fold(&a.name)))
                .map(|a| Group {
                    title: a.name.clone(),
                    subtitle: match a.albums {
                        0 => plural(a.tracks.len(), "morceau", "morceaux"),
                        n => format!(
                            "{} · {}",
                            plural(n, "album", "albums"),
                            plural(a.tracks.len(), "morceau", "morceaux")
                        ),
                    },
                    cover: a.tracks.iter().find_map(|&i| data.tracks[i].cover.clone()),
                    tracks: Arc::new(a.tracks.clone()),
                    // Pas de fiche pour le groupe des morceaux sans artiste.
                    info: (a.name != ondes_library::model::UNKNOWN_ARTIST).then(|| {
                        Target::Artist {
                            id: None,
                            name: a.name.clone(),
                        }
                    }),
                })
                .collect(),
        };
        cx.notify();
    }

    fn open(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group) = self.groups.get(ix) else {
            return;
        };
        let (title, subtitle, cover, info) = (
            group.title.clone(),
            group.subtitle.clone(),
            group.cover.clone(),
            group.info.clone(),
        );
        let scope = Scope::Some(group.tracks.clone());
        let player = self.player.clone();
        let table = cx.new(|cx| TrackTable::new(player, scope, false, window, cx));
        self.detail = Some((title, subtitle, cover, table, info));
        cx.notify();
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let theme = cx.theme().clone();
        range
            .filter_map(|ix| {
                let g = self.groups.get(ix)?;
                Some(
                    h_flex()
                        .id(ElementId::NamedInteger("group".into(), ix as u64))
                        .h(px(GROUP_ROW))
                        .w_full()
                        .px_3()
                        .gap_3()
                        .items_center()
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .hover(|d| d.bg(theme.accent))
                        .on_click(cx.listener(move |this, _, window, cx| this.open(ix, window, cx)))
                        .child(cover(g.cover.clone(), 48., self.kind, &theme))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_medium()
                                        .truncate()
                                        .child(g.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .truncate()
                                        .child(g.subtitle.clone()),
                                ),
                        ),
                )
            })
            .collect()
    }
}

fn cover(
    path: Option<PathBuf>,
    size: f32,
    kind: GroupKind,
    theme: &gpui_kit::component::theme::ThemeColor,
) -> Div {
    let icon = match kind {
        GroupKind::Albums => IconName::DiscAlbum,
        GroupKind::Artists => IconName::MicVocal,
    };
    let fallback = theme.muted_foreground;
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(6.))
        .overflow_hidden()
        .bg(theme.muted)
        .text_color(theme.muted_foreground)
        .child(match path {
            Some(path) => img(path)
                .size_full()
                .object_fit(ObjectFit::Cover)
                .with_fallback(move || Icon::new(icon).text_color(fallback).into_any_element())
                .into_any_element(),
            None => Icon::new(icon).into_any_element(),
        })
}

impl Render for LibraryGroups {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.library.read(cx).folders.is_empty() {
            return empty_library(cx);
        }
        let theme = cx.theme().clone();

        if let Some((title, subtitle, cover_path, table, info)) = &self.detail {
            let info = info.clone();
            let table_for_play = table.clone();
            let table_for_shuffle = table.clone();
            return v_flex()
                .size_full()
                .min_h_0()
                .gap_3()
                .child(
                    h_flex()
                        .gap_4()
                        .items_center()
                        .child(
                            Button::new("back")
                                .icon(IconName::ArrowLeft)
                                .ghost()
                                .tooltip("Retour")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.detail = None;
                                    cx.notify();
                                })),
                        )
                        .child(cover(cover_path.clone(), 96., self.kind, &theme))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_semibold()
                                        .truncate()
                                        .child(title.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(subtitle.clone()),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .pt_1()
                                        .child(
                                            Button::new("detail-play")
                                                .icon(IconName::Play)
                                                .label("Lire")
                                                .primary()
                                                .small()
                                                .on_click(move |_, _, cx| {
                                                    table_for_play.update(cx, |t, cx| {
                                                        t.play_from(0, false, cx)
                                                    });
                                                }),
                                        )
                                        .child(
                                            Button::new("detail-shuffle")
                                                .icon(IconName::Shuffle)
                                                .label("Aléatoire")
                                                .outline()
                                                .small()
                                                .on_click(move |_, _, cx| {
                                                    table_for_shuffle.update(cx, |t, cx| {
                                                        t.play_from(0, true, cx)
                                                    });
                                                }),
                                        )
                                        .when_some(info, |row, target| {
                                            row.child(
                                                Button::new("detail-info")
                                                    .icon(IconName::Info)
                                                    .label("Infos")
                                                    .ghost()
                                                    .small()
                                                    .on_click(move |_, _, cx| {
                                                        crate::info::show(target.clone(), cx)
                                                    }),
                                            )
                                        }),
                                ),
                        ),
                )
                .child(table.clone())
                .into_any_element();
        }

        let count = self.groups.len();
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(search_field(&self.search))
            .child(div().text_xs().text_color(theme.muted_foreground).child(
                match (self.kind, count) {
                    (GroupKind::Albums, n) => format!("{n} album{}", if n > 1 { "s" } else { "" }),
                    (GroupKind::Artists, n) => {
                        format!("{n} artiste{}", if n > 1 { "s" } else { "" })
                    }
                },
            ))
            .child(
                uniform_list(
                    "groups",
                    count,
                    cx.processor(|this, range: Range<usize>, _, cx| this.render_rows(range, cx)),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .min_h_0(),
            )
            .into_any_element()
    }
}

// ─── Page Dossiers ─────────────────────────────────────────────────────────

pub struct LibraryFolders {
    library: Entity<LibraryModel>,
    _subscriptions: Vec<Subscription>,
}

impl LibraryFolders {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let library = LibraryModel::global(cx);
        Self {
            _subscriptions: vec![cx.observe(&library, |_, _, cx| cx.notify())],
            library,
        }
    }
}

impl Render for LibraryFolders {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let lib = self.library.read(cx);
        if lib.folders.is_empty() {
            return empty_library(cx);
        }

        let status: AnyElement = match (&lib.scanning, &lib.error, &lib.last_report) {
            (Some(progress), _, _) => {
                let (text, value) = match progress {
                    ScanProgress::Listing { found } => {
                        (format!("Recherche des fichiers… {found} trouvés"), None)
                    }
                    ScanProgress::Reading { done, total } => (
                        format!("Lecture des tags… {done} / {total}"),
                        Some(if *total == 0 {
                            100.
                        } else {
                            *done as f32 * 100. / *total as f32
                        }),
                    ),
                };
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(text),
                    )
                    .child(match value {
                        Some(v) => Progress::new("scan-progress").value(v).into_any_element(),
                        None => Progress::new("scan-progress")
                            .loading(true)
                            .into_any_element(),
                    })
                    .into_any_element()
            }
            (None, Some(err), _) => div()
                .text_xs()
                .text_color(theme.danger)
                .child(format!("Erreur de scan : {err}"))
                .into_any_element(),
            (None, None, Some(r)) => div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!(
                    "{} morceaux · dernier scan : {} ajouté(s), {} mis à jour, {} retiré(s)",
                    r.total, r.added, r.updated, r.removed
                ))
                .into_any_element(),
            _ => div().into_any_element(),
        };

        let folders: Vec<(PathBuf, usize)> = lib
            .folders
            .iter()
            .map(|f| (f.clone(), lib.count_in(f)))
            .collect();
        let scanning = lib.scanning.is_some();

        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("add-folder")
                            .icon(IconName::FolderPlus)
                            .label("Ajouter un dossier…")
                            .outline()
                            .on_click(|_, window, cx| pick_folder(window, cx)),
                    )
                    .child(
                        Button::new("rescan")
                            .icon(IconName::RefreshCw)
                            .label("Rescanner")
                            .outline()
                            .disabled(scanning)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.library.update(cx, |l, cx| l.rescan(cx));
                            })),
                    ),
            )
            .child(status)
            .children(folders.into_iter().enumerate().map(|(ix, (folder, count))| {
                let library = self.library.clone();
                let to_remove = folder.clone();
                h_flex()
                    .id(ElementId::NamedInteger("folder".into(), ix as u64))
                    .gap_3()
                    .px_3()
                    .py_2()
                    .items_center()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .child(Icon::new(IconName::Folder).text_color(theme.muted_foreground))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().truncate().child(folder.display().to_string()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{count} morceau{}", if count > 1 { "x" } else { "" })),
                            ),
                    )
                    .child(
                        Button::new("remove-folder")
                            .icon(IconName::Trash)
                            .ghost()
                            .small()
                            .tooltip("Retirer de la bibliothèque (les fichiers ne sont pas supprimés)")
                            .on_click(move |_, window, cx| {
                                if let Err(e) = library.update(cx, |l, cx| l.remove_folder(&to_remove, cx)) {
                                    window.push_notification(Notification::error(e), cx);
                                }
                            }),
                    )
            }))
            .into_any_element()
    }
}
