//! Radios favorites : liste, filtre, ajout manuel, import de playlist.

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, WindowExt as _,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ondes_radio::Station;

use crate::logos::Logos;
use crate::player::Player;
use crate::radio::FavoritesModel;
use crate::radio_explore::toggle_favorite;
use crate::station_row::StationRow;

pub struct RadioFavorites {
    player: Entity<Player>,
    favorites: Entity<FavoritesModel>,
    filter: Entity<InputState>,
    /// Formulaire d'ajout manuel, affiché à la demande.
    adding: bool,
    new_name: Entity<InputState>,
    new_url: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl RadioFavorites {
    pub fn new(player: Entity<Player>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let favorites = FavoritesModel::global(cx);
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filtrer les favoris…"));
        let new_name = cx.new(|cx| InputState::new(window, cx).placeholder("Nom de la station"));
        let new_url =
            cx.new(|cx| InputState::new(window, cx).placeholder("URL du flux (http…, .pls, .m3u)"));
        let logos = Logos::global(cx);
        let subscriptions = vec![
            cx.subscribe(&filter, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe_in(
                &new_url,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.submit(window, cx);
                    }
                },
            ),
            cx.observe(&favorites, |_, _, cx| cx.notify()),
            cx.observe(&logos, |_, _, cx| cx.notify()),
            cx.observe(&player, |_, _, cx| cx.notify()),
        ];
        Self {
            player,
            favorites,
            filter,
            adding: false,
            new_name,
            new_url,
            _subscriptions: subscriptions,
        }
    }

    fn visible(&self, cx: &App) -> Vec<Station> {
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        self.favorites
            .read(cx)
            .stations()
            .iter()
            .filter(|s| {
                needle.is_empty()
                    || s.name.to_lowercase().contains(&needle)
                    || s.tags.iter().any(|t| t.to_lowercase().contains(&needle))
            })
            .cloned()
            .collect()
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.new_url.read(cx).value().trim().to_string();
        let name = self.new_name.read(cx).value().trim().to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            window.push_notification(
                Notification::warning("L'URL doit commencer par http:// ou https://"),
                cx,
            );
            return;
        }
        let name = if name.is_empty() {
            ondes_audio::SourceSpec::Url(url.clone()).label()
        } else {
            name
        };
        match self
            .favorites
            .update(cx, |f, cx| f.add(&Station::manual(name, url), cx))
        {
            Ok(()) => {
                self.adding = false;
                self.new_name
                    .update(cx, |s, cx| s.set_value("", window, cx));
                self.new_url.update(cx, |s, cx| s.set_value("", window, cx));
            }
            Err(e) => window.push_notification(Notification::error(e), cx),
        }
        cx.notify();
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Importer".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                for path in paths {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let note = match this.favorites.update(cx, |f, cx| f.import(&path, cx)) {
                        Ok((0, _)) => {
                            Notification::warning(format!("{name} : aucune station trouvée"))
                        }
                        Ok((read, added)) => Notification::success(format!(
                            "{name} : {read} station(s) lue(s), {added} ajoutée(s)"
                        )),
                        Err(e) => Notification::error(format!("{name} : {e}")),
                    };
                    window.push_notification(note, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let stations = self.visible(cx);
        let logos = Logos::global(cx);
        let current = self.player.read(cx).station.as_ref().map(|s| s.url.clone());
        range
            .filter_map(|ix| {
                let station = stations.get(ix)?.clone();
                let logo = station
                    .favicon
                    .clone()
                    .and_then(|url| logos.update(cx, |l, cx| l.get(&url, cx)));
                let player = self.player.clone();
                let to_play = station.clone();
                let favs = self.favorites.clone();
                let to_toggle = station.clone();
                Some(
                    StationRow {
                        id: ElementId::NamedInteger("favorite".into(), ix as u64),
                        current: current.as_deref() == Some(station.url.as_str()),
                        station,
                        logo,
                        favorite: true,
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

impl Render for RadioFavorites {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let total = self.favorites.read(cx).stations().len();
        let available = self.favorites.read(cx).is_available();
        let count = self.visible(cx).len();

        v_flex()
            .size_full()
            .min_h_0()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div().flex_1().child(
                            Input::new(&self.filter)
                                .prefix(Icon::new(IconName::Search).small())
                                .cleanable(true),
                        ),
                    )
                    .child(
                        Button::new("add-station")
                            .icon(IconName::Plus)
                            .label("Ajouter une station")
                            .outline()
                            .disabled(!available)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.adding = !this.adding;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("import")
                            .icon(IconName::FileUp)
                            .label("Importer une playlist…")
                            .outline()
                            .disabled(!available)
                            .on_click(cx.listener(|this, _, window, cx| this.import(window, cx))),
                    ),
            )
            .when(self.adding, |col| {
                col.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .p_3()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.border)
                        .child(div().w(px(220.)).child(Input::new(&self.new_name)))
                        .child(div().flex_1().child(Input::new(&self.new_url)))
                        .child(
                            Button::new("submit-station")
                                .label("Ajouter")
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                        )
                        .child(
                            Button::new("cancel-station")
                                .label("Annuler")
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.adding = false;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(if !available {
                        "Base de favoris indisponible (voir les journaux)".to_string()
                    } else if count == total {
                        format!("{total} favori(s)")
                    } else {
                        format!("{count} sur {total} favori(s)")
                    }),
            )
            .when(total == 0, |col| {
                col.child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(IconName::Heart).large())
                        .child(div().text_sm().child(
                            "Aucune favorite pour l'instant : utilise ♡ dans Explorer, ou importe une playlist.",
                        )),
                )
            })
            .when(total > 0, |col| {
                col.child(
                    uniform_list(
                        "favorite-stations",
                        count,
                        cx.processor(|this, range: Range<usize>, _, cx| this.render_rows(range, cx)),
                    )
                    .flex_1()
                    .min_h_0(),
                )
            })
    }
}
