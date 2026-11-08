//! Vue compacte : petite fenêtre du lecteur, la fenêtre principale réduite.
//!
//! Quatre modes d'affichage : pochette (ou logo de la radio), spectre, titre
//! en grand, file d'attente. Le mode, la taille et la position sont
//! mémorisés.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    slider::{Slider, SliderEvent, SliderState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::logos::Logos;
use crate::player::Player;
use crate::player_bar::{Status, clock, status};
use crate::radio::FavoritesModel;
use crate::radio_explore::toggle_favorite;
use crate::settings::{Settings, WindowState};
use crate::spectrum::Spectrum;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Cover,
    Spectrum,
    Title,
    Queue,
}

impl Mode {
    const ALL: [Mode; 4] = [Mode::Cover, Mode::Spectrum, Mode::Title, Mode::Queue];

    fn key(self) -> &'static str {
        match self {
            Mode::Cover => "cover",
            Mode::Spectrum => "spectrum",
            Mode::Title => "title",
            Mode::Queue => "queue",
        }
    }

    fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|m| m.key() == key)
            .unwrap_or(Mode::Cover)
    }

    fn icon(self) -> IconName {
        match self {
            Mode::Cover => IconName::Image,
            Mode::Spectrum => IconName::AudioLines,
            Mode::Title => IconName::Type,
            Mode::Queue => IconName::ListMusic,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Mode::Cover => "Pochette",
            Mode::Spectrum => "Spectre",
            Mode::Title => "Titre",
            Mode::Queue => "File d'attente",
        }
    }
}

/// Ouvre la vue compacte et réduit la fenêtre principale `main`.
pub fn open(
    player: Entity<Player>,
    main: AnyWindowHandle,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    let saved = Settings::get(cx).compact.window;
    let options = WindowOptions {
        window_bounds: Some(match saved {
            Some(w) => WindowBounds::Windowed(Bounds::new(
                point(px(w.x), px(w.y)),
                size(px(w.width.max(260.)), px(w.height.max(240.))),
            )),
            None => WindowBounds::centered(size(px(340.), px(440.)), cx),
        }),
        window_min_size: Some(size(px(260.), px(240.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Ondes".into()),
            ..Default::default()
        }),
        app_id: Some("ondes".into()),
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| CompactView::new(player, main, window, cx))
    }) {
        Ok((handle, _)) => {
            if let Err(e) = main.update(cx, |_, window, _| window.minimize_window()) {
                tracing::warn!("réduction de la fenêtre principale impossible : {e:#}");
            }
            Some(handle)
        }
        Err(err) => {
            tracing::error!("vue compacte impossible : {err:#}");
            None
        }
    }
}

pub struct CompactView {
    player: Entity<Player>,
    mode: Mode,
    spectrum: Entity<Spectrum>,
    volume: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
}

impl CompactView {
    fn new(
        player: Entity<Player>,
        main: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mode = Mode::from_key(&Settings::get(cx).compact.mode);
        let spectrum = cx.new(|cx| Spectrum::with_size(player.clone(), 28, 150., 6., true, cx));
        let initial = player.read(cx).volume();
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.01)
                .default_value(initial)
        });
        let subscriptions = vec![
            cx.subscribe(&volume, |this, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(value) = event {
                    let v = value.start();
                    this.player.update(cx, |p, cx| p.set_volume(v, cx));
                }
            }),
            cx.observe_in(&player, window, |this, player, window, cx| {
                let volume = player.read(cx).volume();
                if (this.volume.read(cx).value().start() - volume).abs() > 0.001 {
                    this.volume
                        .update(cx, |s, cx| s.set_value(volume, window, cx));
                }
                cx.notify();
            }),
            cx.observe(&FavoritesModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&Logos::global(cx), |_, _, cx| cx.notify()),
            cx.observe_window_bounds(window, |_, window, cx| {
                let b = window.bounds();
                let state = WindowState {
                    x: f32::from(b.origin.x),
                    y: f32::from(b.origin.y),
                    width: f32::from(b.size.width),
                    height: f32::from(b.size.height),
                    maximized: false,
                };
                Settings::update(cx, |s| s.compact.window = Some(state));
            }),
            // Vue fermée (bouton, gestionnaire de fenêtres) : retour à la fenêtre principale.
            cx.on_release(move |_, cx| {
                main.update(cx, |_, window, _| window.activate_window())
                    .ok();
            }),
        ];
        Self {
            player,
            mode,
            spectrum,
            volume,
            _subscriptions: subscriptions,
        }
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        Settings::update(cx, |s| s.compact.mode = mode.key().into());
        cx.notify();
    }

    /// Image du mode « Pochette » : pochette du morceau, ou logo de la radio.
    fn artwork(&self, cx: &mut Context<Self>) -> Option<std::path::PathBuf> {
        let p = self.player.read(cx);
        if let Some(cover) = p.current_track().and_then(|t| t.cover.clone()) {
            return Some(cover);
        }
        let logo = p.station.as_ref()?.favicon.clone()?;
        Logos::global(cx).update(cx, |l, cx| l.get(&logo, cx))
    }

    fn render_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        match self.mode {
            Mode::Cover => {
                let live = self.player.read(cx).track.as_ref().is_some_and(|t| t.live);
                let muted = theme.muted_foreground;
                let fallback = move || {
                    Icon::new(if live {
                        IconName::Radio
                    } else {
                        IconName::Music
                    })
                    .size(px(64.))
                    .text_color(muted)
                    .into_any_element()
                };
                let image = match self.artwork(cx) {
                    Some(path) => img(path)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .with_fallback(fallback)
                        .into_any_element(),
                    None => fallback(),
                };
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(theme.radius_lg)
                    .overflow_hidden()
                    .child(image)
                    .into_any_element()
            }
            Mode::Spectrum => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(self.spectrum.clone())
                .into_any_element(),
            Mode::Title => {
                let p = self.player.read(cx);
                let main = p
                    .now_playing
                    .clone()
                    .filter(|_| p.track.as_ref().is_some_and(|t| t.live))
                    .or_else(|| p.title())
                    .unwrap_or_else(|| "Aucune lecture".into());
                let secondary = if p.track.as_ref().is_some_and(|t| t.live) {
                    p.title().filter(|t| *t != main)
                } else {
                    p.track_subtitle()
                };
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .px_2()
                    .text_center()
                    .child(div().text_2xl().font_semibold().child(main))
                    .children(secondary.map(|s| {
                        div()
                            .text_base()
                            .text_color(theme.muted_foreground)
                            .child(s)
                    }))
                    .into_any_element()
            }
            Mode::Queue => {
                let p = self.player.read(cx);
                if p.queue.is_empty() {
                    return div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("File d'attente vide")
                        .into_any_element();
                }
                let row = |title: String, detail: String, current: bool| {
                    h_flex()
                        .w_full()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded(theme.radius)
                        .when(current, |r| r.bg(theme.accent))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .truncate()
                                        .when(current, |d| d.font_semibold())
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .truncate()
                                        .text_color(theme.muted_foreground)
                                        .child(detail),
                                ),
                        )
                };
                let detail = |t: &ondes_library::Track| {
                    let artist = t.artist.clone().unwrap_or_default();
                    match (artist.is_empty(), t.duration.is_zero()) {
                        (false, false) => format!("{artist} · {}", clock(t.duration)),
                        (false, true) => artist,
                        (true, false) => clock(t.duration),
                        (true, true) => String::new(),
                    }
                };
                v_flex()
                    .id("compact-queue")
                    .size_full()
                    .overflow_y_scroll()
                    .gap_0p5()
                    .children(
                        p.current_track()
                            .map(|t| row(t.title.clone(), detail(t), true)),
                    )
                    .children(
                        p.queue
                            .upcoming(50)
                            .map(|t| row(t.title.clone(), detail(t), false)),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Render for CompactView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.render_body(cx);
        let theme = cx.theme().clone();
        let p = self.player.read(cx);

        let title = p.title().unwrap_or_else(|| "Aucune lecture".into());
        let subtitle = match status(p) {
            Status::Error(text) => div().text_color(theme.danger).child(text),
            Status::Info(text) => div().child(text),
            Status::Hint(text) | Status::Busy(text) => {
                div().text_color(theme.muted_foreground).child(text)
            }
        };
        let live = p.track.as_ref().is_some_and(|t| t.live);
        let active = p.state.is_active();
        let has_source = p.has_source();
        let has_queue = !p.queue.is_empty();
        let has_next = p.queue.has_next();
        let station = p.current_station();
        let favorites = FavoritesModel::global(cx);
        let favorite = station
            .as_ref()
            .is_some_and(|s| favorites.read(cx).contains(&s.url));
        let recording = p.recording.is_some();
        let can_record = p.can_record() || recording;
        let progress = p
            .track
            .as_ref()
            .filter(|t| !t.live)
            .and_then(|t| t.duration)
            .filter(|d| !d.is_zero())
            .map(|d| (p.position, d));

        let header = h_flex()
            .gap_0p5()
            .items_center()
            .children(Mode::ALL.map(|mode| {
                Button::new(mode.key())
                    .icon(mode.icon())
                    .ghost()
                    .xsmall()
                    .selected(self.mode == mode)
                    .tooltip(mode.label())
                    .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx)))
            }))
            .child(div().flex_1())
            .child(
                Button::new("expand")
                    .icon(IconName::Maximize2)
                    .ghost()
                    .xsmall()
                    .tooltip("Fenêtre principale (Ctrl+Maj+M)")
                    .on_click(|_, window, _| window.remove_window()),
            );

        let timeline = progress.map(|(pos, duration)| {
            let fraction = (pos.as_secs_f32() / duration.as_secs_f32()).clamp(0., 1.);
            h_flex()
                .gap_2()
                .items_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(clock(pos))
                .child(
                    div()
                        .flex_1()
                        .h(px(3.))
                        .rounded_full()
                        .bg(theme.muted)
                        .child(
                            div()
                                .h_full()
                                .w(relative(fraction))
                                .rounded_full()
                                .bg(theme.primary),
                        ),
                )
                .child(clock(duration))
        });

        let controls = h_flex()
            .gap_1()
            .items_center()
            .when_some(station, |row, station| {
                let favs = favorites.clone();
                row.child(
                    Button::new("compact-fav")
                        .icon(if favorite {
                            Icon::new(IconName::Heart).text_color(theme.danger)
                        } else {
                            Icon::new(IconName::Heart)
                        })
                        .ghost()
                        .small()
                        .tooltip(if favorite {
                            "Retirer des favoris"
                        } else {
                            "Ajouter aux favoris"
                        })
                        .on_click(move |_, window, cx| {
                            toggle_favorite(&favs, &station, window, cx)
                        }),
                )
            })
            .when(can_record, |row| {
                row.child(
                    Button::new("compact-record")
                        .icon(if recording {
                            Icon::new(IconName::CircleDot).text_color(theme.danger)
                        } else {
                            Icon::new(IconName::Circle)
                        })
                        .ghost()
                        .small()
                        .selected(recording)
                        .tooltip(if recording {
                            "Arrêter l'enregistrement"
                        } else {
                            "Enregistrer le direct"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.player.update(cx, |p, cx| p.toggle_recording(cx));
                        })),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("compact-prev")
                    .icon(IconName::SkipBack)
                    .ghost()
                    .small()
                    .disabled(!has_queue)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.player.update(cx, |p, cx| p.prev(cx));
                    })),
            )
            .child(
                Button::new("compact-play")
                    .icon(if active {
                        IconName::Pause
                    } else {
                        IconName::Play
                    })
                    .primary()
                    .disabled(!has_source)
                    .tooltip(match (active, live) {
                        (true, true) => "Couper le direct",
                        (true, false) => "Pause",
                        (false, _) => "Lecture",
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.player.update(cx, |p, cx| p.toggle_play(cx));
                    })),
            )
            .child(
                Button::new("compact-next")
                    .icon(IconName::SkipForward)
                    .ghost()
                    .small()
                    .disabled(!has_next)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.player.update(cx, |p, cx| p.next(cx));
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .w(px(70.))
                    .child(Slider::new(&self.volume).accessibility_label("Volume")),
            );

        v_flex()
            .size_full()
            .p_3()
            .gap_2()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(header)
            .child(div().flex_1().min_h_0().child(body))
            .child(
                v_flex()
                    .gap_0p5()
                    .child(div().text_sm().font_semibold().truncate().child(title))
                    .child(div().text_xs().truncate().child(subtitle)),
            )
            .children(timeline)
            .child(controls)
    }
}
