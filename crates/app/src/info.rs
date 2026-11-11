//! Page « Infos » : fiches artiste, album et titre (MusicBrainz, Wikipédia,
//! Cover Art Archive), ouvertes depuis la barre de lecture, « En cours »,
//! l'historique ou la bibliothèque. On y navigue d'une fiche à l'autre
//! (titre → album → artiste), avec retour arrière.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    spinner::Spinner,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ondes_info::{
    AlbumInfo, AlbumKind, ArtistInfo, Info, InfoError, ReleaseGroup, TrackInfo, cover_url,
    musicbrainz_search_url, musicbrainz_url,
};
use ondes_library::Track;
use ondes_library::search::fold;

use crate::library::LibraryModel;
use crate::player::Player;
use crate::player_bar::clock;
use crate::runtime::Rt;

/// Ce qu'on veut voir. Sans identifiant MusicBrainz, il est d'abord cherché.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Artist {
        id: Option<String>,
        name: String,
    },
    Album {
        id: Option<String>,
        artist: String,
        title: String,
    },
    Track {
        id: Option<String>,
        artist: String,
        title: String,
        album: Option<String>,
    },
}

impl Target {
    fn label(&self) -> &str {
        match self {
            Target::Artist { name, .. } => name,
            Target::Album { title, .. } | Target::Track { title, .. } => title,
        }
    }

    fn set_id(&mut self, found: String) {
        match self {
            Target::Artist { id, .. } | Target::Album { id, .. } | Target::Track { id, .. } => {
                *id = Some(found)
            }
        }
    }

    /// Recherche équivalente sur le site de MusicBrainz.
    fn search_url(&self) -> String {
        match self {
            Target::Artist { name, .. } => musicbrainz_search_url(name, "artist"),
            Target::Album { artist, title, .. } => {
                musicbrainz_search_url(&format!("{title} {artist}"), "release_group")
            }
            Target::Track { artist, title, .. } => {
                musicbrainz_search_url(&format!("{title} {artist}"), "recording")
            }
        }
    }

    /// Fiche du morceau d'une bibliothèque (il faut un artiste).
    pub fn for_track(track: &Track) -> Option<Self> {
        let artist = track
            .artist
            .clone()
            .or_else(|| track.album_artist.clone())?;
        Some(Target::Track {
            id: None,
            artist,
            title: track.title.clone(),
            album: track.album.clone(),
        })
    }

    /// Fiche d'un titre annoncé par une radio (« Artiste - Titre »).
    pub fn for_announced(text: &str) -> Option<Self> {
        let (artist, title) = text.split_once(" - ")?;
        let (artist, title) = (artist.trim(), title.trim());
        (!artist.is_empty() && !title.is_empty()).then(|| Target::Track {
            id: None,
            artist: artist.to_string(),
            title: title.to_string(),
            album: None,
        })
    }

    /// Ce qui joue : morceau de la bibliothèque ou titre annoncé par la radio.
    pub fn now_playing(player: &Player) -> Option<Self> {
        if let Some(track) = player.current_track() {
            return Self::for_track(track);
        }
        if player.track.as_ref().is_some_and(|t| t.live) {
            return Self::for_announced(player.now_playing.as_deref()?);
        }
        None
    }
}

#[derive(Clone)]
enum Loaded {
    Artist(ArtistInfo),
    Album(AlbumInfo),
    Track(TrackInfo),
}

impl Loaded {
    fn id(&self) -> &str {
        match self {
            Loaded::Artist(a) => &a.id,
            Loaded::Album(a) => &a.id,
            Loaded::Track(t) => &t.id,
        }
    }

    fn musicbrainz(&self) -> String {
        match self {
            Loaded::Artist(a) => musicbrainz_url("artist", &a.id),
            Loaded::Album(a) => musicbrainz_url("release-group", &a.id),
            Loaded::Track(t) => musicbrainz_url("recording", &t.id),
        }
    }
}

enum State {
    Empty,
    Loading,
    Ready(Box<Loaded>),
    NotFound,
    Failed(String),
}

enum Image {
    Pending,
    Missing,
    Ready(PathBuf),
}

/// Service partagé (cache disque dans `~/.cache/ondes/info`).
struct GlobalInfo(Arc<Info>);
impl Global for GlobalInfo {}

/// Page partagée, pour l'ouvrir de partout.
struct GlobalInfoView(WeakEntity<InfoView>);
impl Global for GlobalInfoView {}

pub fn init(cx: &mut App) {
    match Info::new(crate::paths::info_cache()) {
        Ok(info) => cx.set_global(GlobalInfo(Arc::new(info))),
        Err(e) => tracing::error!("infos indisponibles : {e}"),
    }
}

/// Ouvre la fiche (et la page « Infos »).
pub fn show(target: Target, cx: &mut App) {
    if let Some(view) = cx
        .try_global::<GlobalInfoView>()
        .and_then(|v| v.0.upgrade())
    {
        view.update(cx, |v, cx| v.open(target, cx));
    }
}

/// Demande d'affichage de la page, pour la coquille.
pub struct InfoOpened;

pub struct InfoView {
    player: Entity<Player>,
    /// Fiches visitées ; la dernière est affichée.
    stack: Vec<Target>,
    state: State,
    images: HashMap<String, Image>,
    show_others: bool,
    scroll: ScrollHandle,
    /// Biographie de la fiche affichée ; elle arrive après la fiche.
    bio: Option<ondes_info::Bio>,
    _load: Option<Task<()>>,
    _bio: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<InfoOpened> for InfoView {}

impl InfoView {
    pub fn new(player: Entity<Player>, cx: &mut Context<Self>) -> Self {
        cx.set_global(GlobalInfoView(cx.entity().downgrade()));
        let subscriptions = vec![
            // Le bouton « Ce qui joue » suit la lecture ; les marques
            // « dans la bibliothèque » suivent les scans.
            cx.observe(&player, |_, _, cx| cx.notify()),
            cx.observe(&LibraryModel::global(cx), |_, _, cx| cx.notify()),
        ];
        Self {
            player,
            stack: Vec::new(),
            state: State::Empty,
            images: HashMap::new(),
            show_others: false,
            scroll: ScrollHandle::new(),
            bio: None,
            _load: None,
            _bio: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Nouvelle fiche, depuis l'extérieur de la page.
    pub fn open(&mut self, target: Target, cx: &mut Context<Self>) {
        if self.stack.last() != Some(&target) {
            self.stack.push(target);
            self.load(cx);
        }
        cx.emit(InfoOpened);
    }

    /// Fiche de ce qui joue, s'il y a de quoi chercher.
    pub fn open_now_playing(&mut self, cx: &mut Context<Self>) -> bool {
        match Target::now_playing(self.player.read(cx)) {
            Some(target) => {
                self.open(target, cx);
                true
            }
            None => false,
        }
    }

    fn push(&mut self, target: Target, cx: &mut Context<Self>) {
        self.stack.push(target);
        self.load(cx);
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        if self.stack.len() > 1 {
            self.stack.pop();
            self.load(cx);
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.show_others = false;
        self.bio = None;
        self._bio = None;
        self.scroll.set_offset(point(px(0.), px(0.)));
        let Some(target) = self.stack.last().cloned() else {
            self.state = State::Empty;
            cx.notify();
            return;
        };
        let Some(info) = cx.try_global::<GlobalInfo>().map(|g| g.0.clone()) else {
            self.state = State::Failed("service indisponible (voir le journal)".into());
            cx.notify();
            return;
        };
        tracing::info!(?target, "fiche");
        self.state = State::Loading;
        let task = Rt::spawn(cx, async move { fetch(&info, target).await });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.state = match result {
                    Ok(Ok(loaded)) => {
                        // Identifiant trouvé : le retour arrière ne cherche plus.
                        if let Some(top) = this.stack.last_mut() {
                            top.set_id(loaded.id().to_string());
                        }
                        this.load_bio(&loaded, cx);
                        State::Ready(Box::new(loaded))
                    }
                    Ok(Err(InfoError::NotFound)) => State::NotFound,
                    Ok(Err(e)) => {
                        tracing::warn!("infos : {e}");
                        State::Failed(if matches!(e, InfoError::Busy) {
                            "MusicBrainz est surchargé, réessayez dans quelques secondes".into()
                        } else {
                            e.to_string()
                        })
                    }
                    Err(e) => State::Failed(e.to_string()),
                };
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Biographie (Wikipédia) de la fiche : demandée à part, pour afficher
    /// la fiche sans l'attendre.
    fn load_bio(&mut self, loaded: &Loaded, cx: &mut Context<Self>) {
        let wikidata = match loaded {
            Loaded::Artist(a) => a.wikidata.clone(),
            Loaded::Album(a) => a.wikidata.clone(),
            Loaded::Track(_) => None,
        };
        let (Some(wikidata), Some(info)) =
            (wikidata, cx.try_global::<GlobalInfo>().map(|g| g.0.clone()))
        else {
            return;
        };
        let task = Rt::spawn(cx, async move { info.bio(&wikidata).await });
        // Remplacée (donc annulée) dès qu'une autre fiche se charge.
        self._bio = Some(cx.spawn(async move |this, cx| {
            let bio = task.await.ok().flatten();
            this.update(cx, |this, cx| {
                this.bio = bio;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Image du cache, téléchargée au premier affichage.
    fn image(&mut self, url: &str, cx: &mut Context<Self>) -> Option<PathBuf> {
        match self.images.get(url) {
            Some(Image::Ready(path)) => return Some(path.clone()),
            Some(_) => return None,
            None => {}
        }
        let info = cx.try_global::<GlobalInfo>()?.0.clone();
        self.images.insert(url.to_string(), Image::Pending);
        let key = url.to_string();
        let task = Rt::spawn(cx, async move { info.image(&key).await });
        let key = url.to_string();
        cx.spawn(async move |this, cx| {
            let image = match task.await {
                Ok(Ok(Some(path))) => Image::Ready(path),
                _ => Image::Missing,
            };
            this.update(cx, |this, cx| {
                this.images.insert(key, image);
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }
}

async fn fetch(info: &Info, target: Target) -> Result<Loaded, InfoError> {
    match target {
        Target::Artist { id, name } => {
            let id = match id {
                Some(id) => id,
                None => info.find_artist(&name).await?,
            };
            info.artist(&id).await.map(Loaded::Artist)
        }
        Target::Album { id, artist, title } => {
            let id = match id {
                Some(id) => id,
                None => info.find_album(&artist, &title).await?,
            };
            info.album(&id).await.map(Loaded::Album)
        }
        Target::Track {
            id,
            artist,
            title,
            album,
        } => {
            let id = match id {
                Some(id) => id,
                None => info.find_track(&artist, &title, album.as_deref()).await?,
            };
            info.track(&id).await.map(Loaded::Track)
        }
    }
}

// ───────────────────────────────────────────── bibliothèque

fn same(a: &str, b: &str) -> bool {
    fold(a).trim() == fold(b).trim()
}

/// Morceaux de la bibliothèque de cet artiste.
fn library_artist(name: &str, cx: &App) -> Vec<Track> {
    let data = LibraryModel::global(cx).read(cx).data.clone();
    data.artists
        .iter()
        .find(|a| same(&a.name, name))
        .map(|a| a.tracks.iter().map(|&i| data.tracks[i].clone()).collect())
        .unwrap_or_default()
}

/// Morceaux de la bibliothèque de cet album.
fn library_album(artist: &str, title: &str, cx: &App) -> Vec<Track> {
    let data = LibraryModel::global(cx).read(cx).data.clone();
    data.albums
        .iter()
        .find(|a| same(&a.title, title) && a.artist.as_deref().is_none_or(|x| same(x, artist)))
        .map(|a| a.tracks.iter().map(|&i| data.tracks[i].clone()).collect())
        .unwrap_or_default()
}

/// Ce morceau dans la bibliothèque.
fn library_track(artist: &str, title: &str, cx: &App) -> Option<Track> {
    let data = LibraryModel::global(cx).read(cx).data.clone();
    data.tracks
        .iter()
        .find(|t| {
            same(&t.title, title)
                && [t.artist.as_deref(), t.album_artist.as_deref()]
                    .into_iter()
                    .flatten()
                    .any(|a| same(a, artist))
        })
        .cloned()
}

// ───────────────────────────────────────────── mise en forme

const MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];

/// « 1998-04-17 » → « 17 avril 1998 », « 1998-04 » → « avril 1998 ».
fn date_fr(date: &str) -> String {
    let parts: Vec<&str> = date.split('-').collect();
    let month = |m: &str| {
        m.parse::<usize>()
            .ok()
            .and_then(|m| MONTHS.get(m.wrapping_sub(1)))
    };
    match parts.as_slice() {
        [y, m, d] => match (month(m), d.parse::<u32>()) {
            (Some(m), Ok(d)) => format!(
                "{} {m} {y}",
                if d == 1 { "1er".into() } else { d.to_string() }
            ),
            _ => y.to_string(),
        },
        [y, m] => month(m).map_or_else(|| y.to_string(), |m| format!("{m} {y}")),
        [y] => y.to_string(),
        _ => date.to_string(),
    }
}

fn year_of(date: &str) -> &str {
    date.get(..4).unwrap_or(date)
}

/// « depuis 1987 », « 1965 – 2003 ».
fn active(begin: Option<&str>, end: Option<&str>) -> Option<String> {
    match (begin.map(year_of), end.map(year_of)) {
        (Some(b), Some(e)) => Some(format!("{b} – {e}")),
        (Some(b), None) => Some(format!("depuis {b}")),
        (None, Some(e)) => Some(format!("jusqu'en {e}")),
        (None, None) => None,
    }
}

fn length(ms: Option<u64>) -> Option<String> {
    ms.map(|ms| clock(Duration::from_millis(ms)))
}

// ───────────────────────────────────────────── rendu

impl InfoView {
    fn picture(
        &mut self,
        url: Option<String>,
        size: f32,
        fallback: IconName,
        round: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme().clone();
        let path = url.and_then(|u| self.image(&u, cx));
        let frame = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size(px(size))
            .bg(theme.muted)
            .text_color(theme.muted_foreground)
            .overflow_hidden()
            .map(|d| {
                if round {
                    d.rounded_full()
                } else {
                    d.rounded(theme.radius)
                }
            });
        match path {
            Some(path) => frame.child(img(path).size_full().object_fit(ObjectFit::Cover)),
            None => frame.child(Icon::new(fallback).large()),
        }
    }

    fn genres(genres: &[String]) -> Div {
        h_flex().flex_wrap().gap_1().children(
            genres
                .iter()
                .map(|g| Tag::secondary().small().child(g.clone())),
        )
    }

    fn bio(bio: &ondes_info::Bio, cx: &App) -> Div {
        let url = bio.url.clone();
        v_flex()
            .max_w(px(760.))
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(bio.extract.clone()),
            )
            .child(
                h_flex()
                    .child(
                        Button::new("wikipedia")
                            .icon(IconName::ExternalLink)
                            .label(if bio.lang == "fr" {
                                "Lire la suite sur Wikipédia"
                            } else {
                                "Lire la suite sur Wikipédia (en anglais)"
                            })
                            .link()
                            .small()
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("· texte sous licence CC BY-SA"),
                    )
                    .items_center()
                    .gap_1(),
            )
    }

    fn section(title: &str, cx: &App) -> Div {
        div()
            .pt_2()
            .text_base()
            .font_semibold()
            .text_color(cx.theme().foreground)
            .child(title.to_string())
    }

    /// Vignette d'album cliquable.
    fn album_card(
        &mut self,
        ix: usize,
        group: &ReleaseGroup,
        owned: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let cover = self.picture(
            Some(group.cover_url(250)),
            140.,
            IconName::DiscAlbum,
            false,
            cx,
        );
        let target = Target::Album {
            id: Some(group.id.clone()),
            artist: group.artist.clone(),
            title: group.title.clone(),
        };
        let mut meta: Vec<String> = Vec::new();
        if let Some(year) = group.year {
            meta.push(year.to_string());
        }
        if !group.secondary.is_empty() {
            meta.push(group.secondary.clone());
        } else if group.kind != AlbumKind::Album {
            meta.push(group.kind.label().to_string());
        }
        v_flex()
            .id(ElementId::NamedInteger("album-card".into(), ix as u64))
            .w(px(140.))
            .gap_1()
            .cursor_pointer()
            .rounded(theme.radius)
            .hover(|d| d.opacity(0.85))
            .on_click(cx.listener(move |this, _, _, cx| this.push(target.clone(), cx)))
            .child(cover)
            .child(
                div()
                    .text_sm()
                    .font_medium()
                    .truncate()
                    .child(group.title.clone()),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .when(owned, |row| {
                        row.child(
                            div()
                                .text_color(theme.primary)
                                .child(Icon::new(IconName::Check).xsmall()),
                        )
                    })
                    .child(div().truncate().child(meta.join(" · "))),
            )
    }

    fn artist_links(
        &self,
        artists: &[ondes_info::ArtistRef],
        fallback: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let row = h_flex().flex_wrap().gap_1().items_center();
        if artists.is_empty() {
            return row.child(div().text_base().child(fallback.to_string()));
        }
        row.children(artists.iter().enumerate().map(|(i, a)| {
            let target = Target::Artist {
                id: Some(a.id.clone()),
                name: a.name.clone(),
            };
            Button::new(ElementId::NamedInteger("artist-link".into(), i as u64))
                .label(a.name.clone())
                .link()
                .on_click(cx.listener(move |this, _, _, cx| this.push(target.clone(), cx)))
        }))
    }

    fn play_button(&self, id: &'static str, label: &str, tracks: Vec<Track>) -> Button {
        let player = self.player.clone();
        Button::new(id)
            .icon(IconName::Play)
            .label(label.to_string())
            .primary()
            .small()
            .on_click(move |_, _, cx| {
                let tracks = tracks.clone();
                player.update(cx, |p, cx| p.play_tracks(tracks, 0, cx));
            })
    }

    fn render_artist(&mut self, a: ArtistInfo, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let photo = self.picture(
            self.bio.as_ref().and_then(|b| b.image.clone()),
            140.,
            IconName::MicVocal,
            true,
            cx,
        );
        let mut facts: Vec<String> = Vec::new();
        facts.extend(a.kind.clone());
        if !a.origin.is_empty() {
            facts.push(a.origin.join(", "));
        }
        facts.extend(active(a.begin.as_deref(), a.end.as_deref()));
        let owned_tracks = library_artist(&a.name, cx);
        let owned_albums: Vec<String> = {
            let mut titles: Vec<String> = owned_tracks
                .iter()
                .filter_map(|t| t.album.as_deref().map(fold))
                .collect();
            titles.dedup();
            titles
        };

        let header = h_flex().gap_5().items_center().child(photo).child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(div().text_2xl().font_semibold().child(a.name.clone()))
                .children(
                    a.disambiguation
                        .clone()
                        .map(|d| div().text_sm().text_color(theme.muted_foreground).child(d)),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(facts.join(" · ")),
                )
                .child(Self::genres(&a.genres))
                .when(!owned_tracks.is_empty(), |col| {
                    let n = owned_tracks.len();
                    col.child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(self.play_button("play-artist", "Écouter", owned_tracks.clone()))
                            .child(div().text_xs().text_color(theme.muted_foreground).child(
                                format!(
                                    "{n} morceau{} dans la bibliothèque",
                                    if n > 1 { "x" } else { "" }
                                ),
                            )),
                    )
                }),
        );

        let mut body = v_flex().gap_4().child(header);
        if let Some(bio) = &self.bio {
            body = body.child(Self::bio(bio, cx));
        }
        let sections = [
            (AlbumKind::Album, "Albums"),
            (AlbumKind::Ep, "EP"),
            (
                AlbumKind::Other,
                "Autres parutions (live, compilations, remix…)",
            ),
        ];
        let mut ix = 0;
        for (kind, title) in sections {
            let groups: Vec<ReleaseGroup> = a
                .discography
                .iter()
                .filter(|g| g.kind == kind)
                .cloned()
                .collect();
            if groups.is_empty() {
                continue;
            }
            body = body.child(Self::section(title, cx));
            if kind == AlbumKind::Other && !self.show_others {
                let n = groups.len();
                body = body.child(
                    h_flex().child(
                        Button::new("show-others")
                            .label(format!("Afficher les {n} parutions"))
                            .outline()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_others = true;
                                cx.notify();
                            })),
                    ),
                );
                continue;
            }
            let cards: Vec<Stateful<Div>> = groups
                .iter()
                .map(|g| {
                    ix += 1;
                    let owned = owned_albums.contains(&fold(&g.title));
                    self.album_card(ix, g, owned, cx)
                })
                .collect();
            body = body.child(h_flex().flex_wrap().gap_4().children(cards));
        }
        body
    }

    fn render_album(&mut self, a: AlbumInfo, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let cover = self.picture(Some(a.cover_url(500)), 180., IconName::DiscAlbum, false, cx);
        let mut facts: Vec<String> = vec![if a.secondary.is_empty() {
            a.kind.label().to_string()
        } else {
            a.secondary.clone()
        }];
        facts.extend(a.date.as_deref().map(date_fr));
        facts.extend(a.label.clone());
        let owned = library_album(&a.artist, &a.title, cx);
        let owned_titles: Vec<String> = owned.iter().map(|t| fold(&t.title)).collect();
        let total: u64 = a.tracks.iter().filter_map(|t| t.length_ms).sum();
        if total > 0 {
            facts.push(clock(Duration::from_millis(total)));
        }
        let artist_links = self.artist_links(&a.artists, &a.artist, cx);

        let header = h_flex().gap_5().items_center().child(cover).child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(div().text_2xl().font_semibold().child(a.title.clone()))
                .child(artist_links)
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(facts.join(" · ")),
                )
                .child(Self::genres(&a.genres))
                .when(!owned.is_empty(), |col| {
                    col.child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(self.play_button("play-album", "Écouter l'album", owned.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Dans la bibliothèque"),
                            ),
                    )
                }),
        );

        let rows = a.tracks.iter().enumerate().map(|(i, t)| {
            let have = owned_titles.contains(&fold(&t.title));
            let target = Target::Track {
                id: t.recording_id.clone(),
                artist: a.artist.clone(),
                title: t.title.clone(),
                album: Some(a.title.clone()),
            };
            h_flex()
                .id(ElementId::NamedInteger("album-track".into(), i as u64))
                .h(px(34.))
                .px_2()
                .gap_3()
                .items_center()
                .rounded(theme.radius)
                .cursor_pointer()
                .hover(|d| d.bg(theme.accent))
                .on_click(cx.listener(move |this, _, _, cx| this.push(target.clone(), cx)))
                .child(
                    div()
                        .w(px(36.))
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t.position.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .truncate()
                        .child(t.title.clone()),
                )
                .when(have, |row| {
                    row.child(
                        div()
                            .text_color(theme.primary)
                            .child(Icon::new(IconName::Check).xsmall()),
                    )
                })
                .child(
                    div()
                        .w(px(48.))
                        .text_xs()
                        .text_right()
                        .text_color(theme.muted_foreground)
                        .child(length(t.length_ms).unwrap_or_default()),
                )
        });

        let mut body = v_flex().gap_4().child(header);
        if let Some(bio) = &self.bio {
            body = body.child(Self::bio(bio, cx));
        }
        if !a.tracks.is_empty() {
            body = body
                .child(Self::section("Pistes", cx))
                .child(v_flex().max_w(px(760.)).children(rows));
        }
        body
    }

    fn render_track(&mut self, t: TrackInfo, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let cover_id = t.albums.first().map(|g| g.id.clone());
        let cover = self.picture(
            cover_id.map(|id| cover_url(&id, 500)),
            180.,
            IconName::Music,
            false,
            cx,
        );
        let mut facts: Vec<String> = Vec::new();
        facts.extend(length(t.length_ms));
        if let Some(date) = &t.first_release {
            facts.push(format!("première sortie : {}", date_fr(date)));
        }
        let owned = library_track(&t.artist, &t.title, cx);
        let artist_links = self.artist_links(&t.artists, &t.artist, cx);

        let header = h_flex().gap_5().items_center().child(cover).child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(div().text_2xl().font_semibold().child(t.title.clone()))
                .child(artist_links)
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(facts.join(" · ")),
                )
                .child(Self::genres(&t.genres))
                .when_some(owned, |col, track| {
                    col.child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(self.play_button("play-track", "Écouter", vec![track]))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Dans la bibliothèque"),
                            ),
                    )
                }),
        );

        let mut body = v_flex().gap_4().child(header);
        if !t.credits.is_empty() {
            body = body.child(Self::section("Crédits", cx)).child(
                v_flex()
                    .max_w(px(760.))
                    .gap_1()
                    .children(t.credits.iter().map(|c| {
                        h_flex()
                            .gap_3()
                            .items_start()
                            .child(
                                div()
                                    .w(px(150.))
                                    .flex_shrink_0()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(c.role.clone()),
                            )
                            .child(div().flex_1().text_sm().child(c.names.join(", ")))
                    })),
            );
        }
        if !t.albums.is_empty() {
            let shown: Vec<ReleaseGroup> = t.albums.iter().take(12).cloned().collect();
            let cards: Vec<Stateful<Div>> = shown
                .iter()
                .enumerate()
                .map(|(i, g)| self.album_card(i, g, false, cx))
                .collect();
            body = body
                .child(Self::section("Paru sur", cx))
                .child(h_flex().flex_wrap().gap_4().children(cards));
        }
        body
    }

    fn message(&self, icon: IconName, text: String, cx: &App) -> Div {
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(icon).large())
            .child(div().text_sm().text_center().max_w(px(520.)).child(text))
    }
}

impl Render for InfoView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let can_play_now = Target::now_playing(self.player.read(cx)).is_some();
        let top = self.stack.last().cloned();

        // Fil d'Ariane : les trois dernières fiches.
        let trail: Vec<String> = self
            .stack
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|t| t.label().to_string())
            .collect();
        let external = match &self.state {
            State::Ready(loaded) => Some(loaded.musicbrainz()),
            _ => top.as_ref().map(Target::search_url),
        };
        let toolbar = h_flex()
            .gap_2()
            .items_center()
            .when(self.stack.len() > 1, |row| {
                row.child(
                    Button::new("info-back")
                        .icon(IconName::ArrowLeft)
                        .ghost()
                        .small()
                        .tooltip("Fiche précédente")
                        .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(trail.join("  ›  ")),
            )
            .when(can_play_now, |row| {
                row.child(
                    Button::new("info-now")
                        .icon(IconName::Disc3)
                        .label("Ce qui joue")
                        .ghost()
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_now_playing(cx);
                        })),
                )
            })
            .when_some(external, |row, url| {
                row.child(
                    Button::new("info-musicbrainz")
                        .icon(IconName::ExternalLink)
                        .label("MusicBrainz")
                        .ghost()
                        .small()
                        .tooltip("Ouvrir sur musicbrainz.org (corriger, compléter)")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
            });

        let content: AnyElement = match &self.state {
            State::Empty => self
                .message(
                    IconName::Info,
                    "Rien à afficher pour l'instant : lancez un morceau ou une radio qui \
                     annonce ses titres, ou ouvrez les infos depuis la bibliothèque ou \
                     l'historique."
                        .into(),
                    cx,
                )
                .into_any_element(),
            State::Loading => h_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .gap_3()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(Spinner::new())
                .child(format!(
                    "Recherche de « {} »…",
                    top.as_ref().map(Target::label).unwrap_or_default()
                ))
                .into_any_element(),
            State::NotFound => self
                .message(
                    IconName::SearchX,
                    format!(
                        "« {} » est introuvable sur MusicBrainz. Le lien MusicBrainz \
                         ci-dessus lance la recherche sur le site.",
                        top.as_ref().map(Target::label).unwrap_or_default()
                    ),
                    cx,
                )
                .into_any_element(),
            State::Failed(e) => self
                .message(IconName::CloudOff, format!("Infos indisponibles : {e}"), cx)
                .child(
                    Button::new("info-retry")
                        .icon(IconName::RefreshCw)
                        .label("Réessayer")
                        .outline()
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                )
                .into_any_element(),
            State::Ready(loaded) => {
                let body = match (**loaded).clone() {
                    Loaded::Artist(a) => self.render_artist(a, cx),
                    Loaded::Album(a) => self.render_album(a, cx),
                    Loaded::Track(t) => self.render_track(t, cx),
                };
                div()
                    .id("info-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(body.pb_6())
                    .into_any_element()
            }
        };

        v_flex()
            .size_full()
            .min_h_0()
            .gap_3()
            .child(toolbar)
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    // Pas de `super::*` : il importerait la macro `test` de GPUI.
    use super::{Target, active, date_fr};

    #[test]
    fn french_dates() {
        assert_eq!(date_fr("1998-04-17"), "17 avril 1998");
        assert_eq!(date_fr("2001-01-01"), "1er janvier 2001");
        assert_eq!(date_fr("1998-12"), "décembre 1998");
        assert_eq!(date_fr("1998"), "1998");
        assert_eq!(active(Some("1987"), None).as_deref(), Some("depuis 1987"));
        assert_eq!(
            active(Some("1965-03-02"), Some("2003-09-12")).as_deref(),
            Some("1965 – 2003")
        );
    }

    #[test]
    fn announced_titles() {
        assert_eq!(
            Target::for_announced("Massive Attack - Teardrop"),
            Some(Target::Track {
                id: None,
                artist: "Massive Attack".into(),
                title: "Teardrop".into(),
                album: None,
            })
        );
        assert_eq!(Target::for_announced("Le journal de 13h"), None);
    }
}
