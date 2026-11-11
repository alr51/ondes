//! Historique d'écoute : modèle partagé (copie en mémoire de la table
//! SQLite) et page « Historique ».
//!
//! Le lecteur y inscrit chaque titre annoncé par une radio et chaque
//! fichier écouté au moins [`LISTENED`] (ou la moitié s'il est plus court).

use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, TimeZone};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, StyledExt, WindowExt as _,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ondes_audio::SourceSpec;
use ondes_library::search::{Query, fold};
use ondes_radio::{History, HistoryEntry, HistoryKind, Station};

use crate::library::LibraryModel;
use crate::logos::Logos;
use crate::player::Player;
use crate::radio::FavoritesModel;
use crate::settings::Settings;

/// Durée d'écoute à partir de laquelle un fichier entre dans l'historique.
pub const LISTENED: Duration = Duration::from_secs(30);
/// Entrées chargées au démarrage (la base en garde autant).
const LOADED: usize = 10_000;
/// Hauteur fixe des lignes : requise par la liste virtualisée.
const ROW_HEIGHT: f32 = 44.;
/// Délai pour confirmer l'effacement.
const CONFIRM: Duration = Duration::from_secs(4);

pub struct HistoryModel {
    store: Option<History>,
    /// Les plus récentes d'abord.
    entries: Vec<HistoryEntry>,
    /// Texte de recherche replié de chaque entrée.
    haystacks: Vec<String>,
}

struct GlobalHistory(Entity<HistoryModel>);
impl Global for GlobalHistory {}

impl HistoryModel {
    pub fn init(cx: &mut App) {
        let path = crate::paths::database();
        let store = match History::open(&path) {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::error!(path = %path.display(), "historique indisponible : {e}");
                None
            }
        };
        let entries = match store.as_ref().map(|s| s.recent(LOADED)) {
            Some(Ok(entries)) => entries,
            Some(Err(e)) => {
                tracing::error!("lecture de l'historique : {e}");
                Vec::new()
            }
            None => Vec::new(),
        };
        let haystacks = entries.iter().map(haystack).collect();
        let entity = cx.new(|_| Self {
            store,
            entries,
            haystacks,
        });
        cx.set_global(GlobalHistory(entity));
    }

    pub fn global(cx: &App) -> Entity<HistoryModel> {
        cx.global::<GlobalHistory>().0.clone()
    }

    pub fn is_available(&self) -> bool {
        self.store.is_some()
    }

    /// Inscrit une écoute (sauf historique désactivé ou répétition).
    pub fn record(&mut self, entry: HistoryEntry, cx: &mut Context<Self>) {
        if !Settings::get(cx).history {
            return;
        }
        let Some(store) = &self.store else {
            return;
        };
        match store.add(&entry) {
            Ok(Some(entry)) => {
                self.haystacks.insert(0, haystack(&entry));
                self.entries.insert(0, entry);
                cx.notify();
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("historique : {e}"),
        }
    }

    pub fn remove(&mut self, id: i64, cx: &mut Context<Self>) {
        if let Some(store) = &self.store
            && let Err(e) = store.remove(id)
        {
            tracing::warn!("historique : {e}");
            return;
        }
        if let Some(ix) = self.entries.iter().position(|e| e.id == id) {
            self.entries.remove(ix);
            self.haystacks.remove(ix);
        }
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if let Some(store) = &self.store {
            store.clear().map_err(|e| e.to_string())?;
        }
        self.entries.clear();
        self.haystacks.clear();
        cx.notify();
        Ok(())
    }
}

fn haystack(e: &HistoryEntry) -> String {
    let parts = [
        Some(e.title.as_str()),
        e.artist.as_deref(),
        e.source.as_deref(),
    ];
    fold(
        &parts
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Maintenant, en secondes Unix.
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Ligne de la liste : en-tête de jour ou écoute (indice dans le modèle).
#[derive(Clone)]
enum Row {
    Day(SharedString),
    Entry(usize),
}

pub struct HistoryView {
    player: Entity<Player>,
    model: Entity<HistoryModel>,
    filter: Entity<InputState>,
    rows: Vec<Row>,
    /// Matches affichés (sans les en-têtes).
    shown: usize,
    /// Premier clic sur « Effacer » : le second, dans le délai, confirme.
    confirm_clear: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl HistoryView {
    pub fn new(player: Entity<Player>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = HistoryModel::global(cx);
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Chercher un titre, un artiste, une radio…")
        });
        let logos = Logos::global(cx);
        let subscriptions = vec![
            cx.subscribe(&filter, |this, _, _: &InputEvent, cx| {
                this.rebuild(cx);
                cx.notify();
            }),
            cx.observe(&model, |this, _, cx| {
                this.rebuild(cx);
                cx.notify();
            }),
            cx.observe(&logos, |_, _, cx| cx.notify()),
            cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()),
        ];
        let mut view = Self {
            player,
            model,
            filter,
            rows: Vec::new(),
            shown: 0,
            confirm_clear: None,
            _subscriptions: subscriptions,
        };
        view.rebuild(cx);
        view
    }

    /// Filtre et regroupe par jour.
    fn rebuild(&mut self, cx: &App) {
        let query = Query::new(&self.filter.read(cx).value());
        let model = self.model.read(cx);
        let today = Local::now().date_naive();
        let mut rows = Vec::new();
        let mut day = None;
        let mut shown = 0;
        for (ix, entry) in model.entries.iter().enumerate() {
            if !query.is_empty() && !query.matches(&model.haystacks[ix]) {
                continue;
            }
            let date = local(entry.played_at).date_naive();
            if day != Some(date) {
                day = Some(date);
                rows.push(Row::Day(day_label(date, today).into()));
            }
            rows.push(Row::Entry(ix));
            shown += 1;
        }
        self.rows = rows;
        self.shown = shown;
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_clear.take().is_none() {
            self.confirm_clear = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CONFIRM).await;
                this.update(cx, |this, cx| {
                    this.confirm_clear = None;
                    cx.notify();
                })
                .ok();
            }));
        } else {
            match self.model.update(cx, |m, cx| m.clear(cx)) {
                Ok(()) => window.push_notification(Notification::info("Historique effacé"), cx),
                Err(e) => window.push_notification(Notification::error(e), cx),
            }
        }
        cx.notify();
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().clone();
        let logos = Logos::global(cx);
        range
            .filter_map(|ix| {
                let row = self.rows.get(ix)?.clone();
                let entry = match row {
                    Row::Day(label) => {
                        return Some(
                            div()
                                .h(px(ROW_HEIGHT))
                                .flex()
                                .items_end()
                                .px_3()
                                .pb_1()
                                .text_xs()
                                .font_semibold()
                                .text_color(theme.muted_foreground)
                                .child(label)
                                .into_any_element(),
                        );
                    }
                    Row::Entry(i) => self.model.read(cx).entries.get(i)?.clone(),
                };
                let logo = entry
                    .favicon
                    .clone()
                    .and_then(|url| logos.update(cx, |l, cx| l.get(&url, cx)));
                Some(
                    self.render_entry(ix, entry, logo, &theme)
                        .into_any_element(),
                )
            })
            .collect()
    }

    fn render_entry(
        &self,
        ix: usize,
        entry: HistoryEntry,
        logo: Option<PathBuf>,
        theme: &gpui_kit::component::Theme,
    ) -> Stateful<Div> {
        let fallback = match entry.kind {
            HistoryKind::Radio => IconName::Radio,
            HistoryKind::File => IconName::FileMusic,
        };
        let fallback_color = theme.muted_foreground;
        let icon = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size_8()
            .rounded(theme.radius)
            .bg(theme.muted)
            .text_color(theme.muted_foreground)
            .overflow_hidden()
            .child(match logo {
                Some(path) => img(path)
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .with_fallback(move || {
                        Icon::new(fallback)
                            .text_color(fallback_color)
                            .into_any_element()
                    })
                    .into_any_element(),
                None => Icon::new(fallback).small().into_any_element(),
            });
        let subtitle: Vec<&str> = [entry.artist.as_deref(), entry.source.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        let subtitle = subtitle.join(" · ");
        let time = local(entry.played_at).format("%H:%M").to_string();

        let player = self.player.clone();
        let to_play = entry.clone();
        let text = entry.display_title();
        let model = self.model.clone();
        let id = entry.id;
        let info = entry
            .artist
            .clone()
            .map(|artist| crate::info::Target::Track {
                id: None,
                artist,
                title: entry.title.clone(),
                album: (entry.kind == HistoryKind::File)
                    .then(|| entry.source.clone())
                    .flatten(),
            });
        h_flex()
            .id(ElementId::NamedInteger("history".into(), ix as u64))
            .group("history-row")
            .h(px(ROW_HEIGHT))
            .w_full()
            .px_3()
            .gap_3()
            .items_center()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|row| row.bg(theme.accent))
            .on_click(move |_, _, cx| play(&player, &to_play, cx))
            .child(
                div()
                    .w(px(40.))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(time),
            )
            .child(icon)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().font_medium().truncate().child(entry.title))
                    .when(!subtitle.is_empty(), |col| {
                        col.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .truncate()
                                .child(subtitle),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_1()
                    .invisible()
                    .group_hover("history-row", |s| s.visible())
                    .when_some(info, |row, target| {
                        row.child(
                            Button::new("info")
                                .icon(IconName::Info)
                                .ghost()
                                .small()
                                .tooltip("Infos : titre, album, artiste")
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    crate::info::show(target.clone(), cx);
                                }),
                        )
                    })
                    .child(
                        Button::new("copy")
                            .icon(IconName::Copy)
                            .ghost()
                            .small()
                            .tooltip("Copier « artiste - titre »")
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                                window.push_notification(
                                    Notification::info(format!("Copié : {text}")),
                                    cx,
                                );
                            }),
                    )
                    .child(
                        Button::new("remove")
                            .icon(IconName::Trash)
                            .ghost()
                            .small()
                            .tooltip("Retirer de l'historique")
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                model.update(cx, |m, cx| m.remove(id, cx));
                            }),
                    ),
            )
    }
}

/// Relance la source d'une écoute : la radio, ou le fichier.
fn play(player: &Entity<Player>, entry: &HistoryEntry, cx: &mut App) {
    match entry.kind {
        HistoryKind::Radio => {
            let favorite = FavoritesModel::global(cx)
                .read(cx)
                .stations()
                .iter()
                .find(|s| s.url == entry.location)
                .cloned();
            let station = favorite.unwrap_or_else(|| Station {
                favicon: entry.favicon.clone(),
                ..Station::manual(
                    entry
                        .source
                        .clone()
                        .unwrap_or_else(|| SourceSpec::Url(entry.location.clone()).label()),
                    entry.location.clone(),
                )
            });
            player.update(cx, |p, cx| p.play_station(station, cx));
        }
        HistoryKind::File => {
            let path = PathBuf::from(&entry.location);
            // Avec ses tags si le morceau est dans la bibliothèque.
            let track = LibraryModel::global(cx)
                .read(cx)
                .data
                .tracks
                .iter()
                .find(|t| t.path == path)
                .cloned();
            player.update(cx, |p, cx| match track {
                Some(track) => p.play_tracks(vec![track], 0, cx),
                None => p.load(SourceSpec::File(path), cx),
            });
        }
    }
}

fn local(secs: i64) -> DateTime<Local> {
    Local
        .timestamp_opt(secs, 0)
        .single()
        .unwrap_or_else(Local::now)
}

fn day_label(date: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    const DAYS: [&str; 7] = [
        "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
    ];
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
    match (today - date).num_days() {
        0 => "Aujourd'hui".into(),
        1 => "Hier".into(),
        _ => {
            let mut label = format!(
                "{} {} {}",
                DAYS[date.weekday().num_days_from_monday() as usize],
                date.day(),
                MONTHS[date.month0() as usize]
            );
            if date.year() != today.year() {
                label.push_str(&format!(" {}", date.year()));
            }
            // Majuscule initiale.
            let mut chars = label.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

impl Render for HistoryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let model = self.model.read(cx);
        let total = model.entries.len();
        let available = model.is_available();
        let enabled = Settings::get(cx).history;
        let confirming = self.confirm_clear.is_some();
        let count = self.rows.len();

        let status = if !available {
            "Historique indisponible (voir les journaux)".to_string()
        } else if !enabled {
            "Historique en pause : rien n'est ajouté (palette, Ctrl+K)".to_string()
        } else if self.shown == total {
            format!("{total} écoute(s)")
        } else {
            format!("{} sur {total} écoute(s)", self.shown)
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
                        div().flex_1().child(
                            Input::new(&self.filter)
                                .prefix(Icon::new(IconName::Search).small())
                                .cleanable(true),
                        ),
                    )
                    .child(
                        Button::new("clear-history")
                            .icon(IconName::Trash)
                            .label(if confirming {
                                "Confirmer l'effacement"
                            } else {
                                "Effacer l'historique"
                            })
                            .map(|b| if confirming { b.danger() } else { b.outline() })
                            .disabled(total == 0)
                            .on_click(cx.listener(|this, _, window, cx| this.clear(window, cx))),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(status),
            )
            .when(total == 0, |col| {
                col.child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(IconName::Clock).large())
                        .child(div().text_sm().text_center().child(
                            "Rien pour l'instant : les titres annoncés par les radios \
                             et les morceaux écoutés apparaîtront ici.",
                        )),
                )
            })
            .when(total > 0, |col| {
                col.child(
                    uniform_list(
                        "history",
                        count,
                        cx.processor(|this, range: Range<usize>, _, cx| {
                            this.render_rows(range, cx)
                        }),
                    )
                    .flex_1()
                    .min_h_0(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::day_label;

    #[test]
    fn day_labels() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        assert_eq!(day_label(today, today), "Aujourd'hui");
        assert_eq!(day_label(today.pred_opt().unwrap(), today), "Hier");
        let d = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        assert_eq!(day_label(d, today), "Mardi 3 novembre");
        let d = NaiveDate::from_ymd_opt(2025, 8, 15).unwrap();
        assert_eq!(day_label(d, today), "Vendredi 15 août 2025");
    }
}
