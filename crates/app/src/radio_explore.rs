//! Exploration de l'annuaire Radio Browser : recherche, filtres, liste infinie.

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, WindowExt as _,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
    searchable_list::SearchableListItem,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ondes_radio::{SearchQuery, Station};

use crate::logos::Logos;
use crate::player::Player;
use crate::radio::{Directory, FavoritesModel};
use crate::runtime::Rt;
use crate::station_row::StationRow;

const PAGE: usize = 100;
const DEBOUNCE: Duration = Duration::from_millis(350);

/// Option d'un filtre (pays, langue, genre).
#[derive(Clone)]
pub struct FilterOption {
    label: SharedString,
    value: String,
}

impl SearchableListItem for FilterOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &String {
        &self.value
    }
}

type FilterList = SearchableVec<FilterOption>;

pub struct RadioExplore {
    player: Entity<Player>,
    name: Entity<InputState>,
    country: Entity<SelectState<FilterList>>,
    language: Entity<SelectState<FilterList>>,
    tag: Entity<SelectState<FilterList>>,
    query: SearchQuery,
    results: Vec<Station>,
    seen: HashSet<String>,
    /// Doublons écartés : comptent quand même dans le décalage de pagination.
    duplicates: usize,
    loading: bool,
    has_more: bool,
    error: Option<String>,
    /// Incrémenté à chaque nouvelle recherche : ignore les réponses périmées.
    generation: u64,
    scroll: UniformListScrollHandle,
    _search: Option<Task<()>>,
    _debounce: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl RadioExplore {
    pub fn new(player: Entity<Player>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Rechercher une station…"));
        let select = |window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
            })
        };
        let country = select(window, cx);
        let language = select(window, cx);
        let tag = select(window, cx);

        let favorites = FavoritesModel::global(cx);
        let logos = Logos::global(cx);
        let subscriptions = vec![
            cx.subscribe_in(
                &name,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.debounced_search(cx),
                    InputEvent::PressEnter { .. } => this.search(true, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &country,
                window,
                |this, _, event: &SelectEvent<FilterList>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.query.countrycode = value.clone();
                    this.search(true, cx);
                },
            ),
            cx.subscribe_in(
                &language,
                window,
                |this, _, event: &SelectEvent<FilterList>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.query.language = value.clone();
                    this.search(true, cx);
                },
            ),
            cx.subscribe_in(
                &tag,
                window,
                |this, _, event: &SelectEvent<FilterList>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.query.tag = value.clone();
                    this.search(true, cx);
                },
            ),
            cx.observe(&favorites, |_, _, cx| cx.notify()),
            cx.observe(&logos, |_, _, cx| cx.notify()),
            cx.observe(&player, |_, _, cx| cx.notify()),
        ];

        let mut this = Self {
            player,
            name,
            country,
            language,
            tag,
            query: SearchQuery::default(),
            results: Vec::new(),
            seen: HashSet::new(),
            duplicates: 0,
            loading: false,
            has_more: true,
            error: None,
            generation: 0,
            scroll: UniformListScrollHandle::new(),
            _search: None,
            _debounce: None,
            _subscriptions: subscriptions,
        };
        this.load_filters(window, cx);
        // Sans critère : les stations les plus écoutées.
        this.search(true, cx);
        this
    }

    fn load_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rb) = Directory::get(cx) else {
            return;
        };
        let task = Rt::spawn(cx, async move {
            let (countries, languages, tags) =
                tokio::join!(rb.countries(), rb.languages(), rb.tags());
            (countries, languages, tags)
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok((countries, languages, tags)) = task.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                let set = |state: &Entity<SelectState<FilterList>>,
                           items: Vec<FilterOption>,
                           window: &mut Window,
                           cx: &mut Context<Self>| {
                    state.update(cx, |s, cx| {
                        s.set_items(SearchableVec::new(items), window, cx)
                    });
                };
                if let Ok(list) = countries {
                    let items = list
                        .into_iter()
                        .map(|c| FilterOption {
                            label: format!("{} ({})", c.name, c.stationcount).into(),
                            value: c.code,
                        })
                        .collect();
                    set(&this.country, items, window, cx);
                }
                if let Ok(list) = languages {
                    let items = list
                        .into_iter()
                        .map(|l| FilterOption {
                            label: format!("{} ({})", capitalize(&l.name), l.stationcount).into(),
                            value: l.name,
                        })
                        .collect();
                    set(&this.language, items, window, cx);
                }
                if let Ok(list) = tags {
                    let items = list
                        .into_iter()
                        .map(|t| FilterOption {
                            label: format!("{} ({})", t.name, t.stationcount).into(),
                            value: t.name,
                        })
                        .collect();
                    set(&this.tag, items, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn debounced_search(&mut self, cx: &mut Context<Self>) {
        self._debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            this.update(cx, |this, cx| this.search(true, cx)).ok();
        }));
    }

    /// Nouvelle recherche (`reset`) ou page suivante.
    fn search(&mut self, reset: bool, cx: &mut Context<Self>) {
        let Some(rb) = Directory::get(cx) else {
            self.error = Some("Annuaire Radio Browser indisponible".into());
            cx.notify();
            return;
        };
        if reset {
            self._debounce = None;
            self.generation += 1;
            self.results.clear();
            self.seen.clear();
            self.duplicates = 0;
            self.has_more = true;
            self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        } else if self.loading || !self.has_more {
            return;
        }

        self.query.name = self.name.read(cx).value().to_string();
        let query = SearchQuery {
            offset: self.results.len() + self.duplicates,
            limit: PAGE,
            ..self.query.clone()
        };
        self.loading = true;
        self.error = None;
        let generation = self.generation;
        let task = Rt::spawn(cx, async move { rb.search(&query).await });
        self._search = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(Ok(list)) => {
                        this.has_more = list.len() == PAGE;
                        for station in list {
                            if this.seen.insert(station.url.clone()) {
                                this.results.push(station);
                            } else {
                                this.duplicates += 1;
                            }
                        }
                    }
                    Ok(Err(e)) => this.error = Some(e.to_string()),
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        // Bientôt en bas de la liste : page suivante.
        if range.end + 20 >= self.results.len() && self.has_more && !self.loading {
            let this = cx.entity().downgrade();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| this.search(false, cx)).ok();
            });
        }
        let favorites = FavoritesModel::global(cx);
        let logos = Logos::global(cx);
        let current = self.player.read(cx).station.as_ref().map(|s| s.url.clone());

        range
            .filter_map(|ix| {
                let station = self.results.get(ix)?.clone();
                let logo = station
                    .favicon
                    .clone()
                    .and_then(|url| logos.update(cx, |l, cx| l.get(&url, cx)));
                let favorite = favorites.read(cx).contains(&station.url);
                let player = self.player.clone();
                let to_play = station.clone();
                let favs = favorites.clone();
                let to_toggle = station.clone();
                Some(
                    StationRow {
                        id: ElementId::NamedInteger("station".into(), ix as u64),
                        current: current.as_deref() == Some(station.url.as_str()),
                        station,
                        logo,
                        favorite,
                        on_play: Rc::new(move |_, cx| {
                            player.update(cx, |p, cx| p.play_station(to_play.clone(), cx));
                        }),
                        on_favorite: Rc::new(move |window, cx| {
                            toggle_favorite(&favs, &to_toggle, window, cx);
                        }),
                    }
                    .render(cx),
                )
            })
            .collect()
    }
}

/// Ajoute / retire un favori, avec notification en cas d'erreur.
pub fn toggle_favorite(
    favorites: &Entity<FavoritesModel>,
    station: &Station,
    window: &mut Window,
    cx: &mut App,
) {
    if let Err(e) = favorites.update(cx, |f, cx| f.toggle(station, cx)) {
        window.push_notification(
            gpui_kit::component::notification::Notification::error(e),
            cx,
        );
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl Render for RadioExplore {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let count = self.results.len();

        let status: AnyElement = if let Some(err) = &self.error {
            h_flex()
                .gap_2()
                .items_center()
                .text_xs()
                .text_color(theme.danger)
                .child(err.clone())
                .child(
                    Button::new("retry")
                        .label("Réessayer")
                        .xsmall()
                        .outline()
                        .on_click(cx.listener(|this, _, _, cx| {
                            let reset = this.results.is_empty();
                            this.search(reset, cx);
                        })),
                )
                .into_any_element()
        } else if self.loading {
            h_flex()
                .gap_2()
                .items_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(Spinner::new().xsmall())
                .child("Recherche…")
                .into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(match (count, self.has_more) {
                    (0, _) => "Aucune station ne correspond".to_string(),
                    (1, _) => "1 station".to_string(),
                    (n, true) => format!("{n} stations et plus"),
                    (n, false) => format!("{n} stations"),
                })
                .into_any_element()
        };

        v_flex()
            .size_full()
            .min_h_0()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div().flex_1().min_w(px(180.)).child(
                            Input::new(&self.name)
                                .prefix(Icon::new(IconName::Search).small())
                                .cleanable(true),
                        ),
                    )
                    .child(
                        div().w(px(170.)).child(
                            Select::new(&self.country)
                                .placeholder("Pays")
                                .search_placeholder("Chercher un pays")
                                .cleanable(true)
                                .menu_width(px(260.)),
                        ),
                    )
                    .child(
                        div().w(px(150.)).child(
                            Select::new(&self.language)
                                .placeholder("Langue")
                                .search_placeholder("Chercher une langue")
                                .cleanable(true)
                                .menu_width(px(240.)),
                        ),
                    )
                    .child(
                        div().w(px(150.)).child(
                            Select::new(&self.tag)
                                .placeholder("Genre")
                                .search_placeholder("Chercher un genre")
                                .cleanable(true)
                                .menu_width(px(240.)),
                        ),
                    ),
            )
            .child(status)
            .child(
                uniform_list(
                    "radio-results",
                    count,
                    cx.processor(|this, range: Range<usize>, _, cx| this.render_rows(range, cx)),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .min_h_0(),
            )
            .when(count == 0 && !self.loading && self.error.is_none(), |col| {
                col.child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(IconName::SearchX).large())
                        .child(
                            div()
                                .text_sm()
                                .child("Essaie un autre nom ou retire un filtre."),
                        ),
                )
            })
    }
}
