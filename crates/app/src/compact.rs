//! Vue compacte : petite fenêtre du lecteur, la fenêtre principale réduite.
//!
//! Mêmes modes d'affichage que la page « En cours » (voir [`crate::stage`]).
//! Le mode, la taille et la position sont mémorisés.

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

use crate::player::Player;
use crate::player_bar::{Status, clock, status};
use crate::radio::FavoritesModel;
use crate::radio_explore::toggle_favorite;
use crate::settings::{Settings, WindowState};
use crate::stage::{Place, Stage, mode_bar};

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
    stage: Entity<Stage>,
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
        let stage = cx.new(|cx| Stage::new(player.clone(), Place::Compact, cx));
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
            cx.observe(&stage, |_, _, cx| cx.notify()),
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
            stage,
            volume,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for CompactView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .items_center()
            .child(mode_bar(&self.stage, cx))
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
            .child(div().flex_1().min_h_0().child(self.stage.clone()))
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
