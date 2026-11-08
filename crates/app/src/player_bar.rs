//! Barre de lecture, reliée au modèle [`Player`].

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    slider::{Slider, SliderEvent, SliderState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ondes_audio::PlaybackState;

use crate::player::Player;
use crate::queue::Repeat;
use crate::radio::FavoritesModel;
use crate::radio_explore::toggle_favorite;
use crate::spectrum::Spectrum;

pub struct PlayerBar {
    player: Entity<Player>,
    volume: Entity<SliderState>,
    /// Progression, en fraction de la durée (0 à 1).
    progress: Entity<SliderState>,
    /// Position affichée pendant que l'utilisateur fait glisser la barre.
    dragging: Option<Duration>,
    spectrum: Entity<Spectrum>,
    _subscriptions: Vec<Subscription>,
}

impl PlayerBar {
    pub fn new(player: Entity<Player>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial = player.read(cx).volume();
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.01)
                .default_value(initial)
        });
        let progress = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.001));
        let subscriptions = vec![
            cx.subscribe(&volume, |this, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(value) = event {
                    let v = value.start();
                    this.player.update(cx, |p, cx| p.set_volume(v, cx));
                }
            }),
            cx.subscribe(&progress, |this, _, event: &SliderEvent, cx| {
                let Some(duration) = this.duration(cx) else {
                    return;
                };
                match event {
                    SliderEvent::Change(v) => {
                        this.dragging = Some(duration.mul_f32(v.start().clamp(0., 1.)));
                    }
                    SliderEvent::Release(v) => {
                        this.dragging = None;
                        let pos = duration.mul_f32(v.start().clamp(0., 1.));
                        this.player.update(cx, |p, cx| p.seek(pos, cx));
                    }
                }
                cx.notify();
            }),
            cx.observe_in(&player, window, |this, player, window, cx| {
                // Volume changé ailleurs (raccourci, MPRIS) : le curseur suit.
                let volume = player.read(cx).volume();
                if (this.volume.read(cx).value().start() - volume).abs() > 0.001 {
                    this.volume
                        .update(cx, |s, cx| s.set_value(volume, window, cx));
                }
                if this.dragging.is_none() {
                    let p = player.read(cx);
                    let fraction = match p.track.as_ref().and_then(|t| t.duration) {
                        Some(d) if !d.is_zero() => {
                            (p.position.as_secs_f32() / d.as_secs_f32()).clamp(0., 1.)
                        }
                        _ => 0.,
                    };
                    this.progress
                        .update(cx, |s, cx| s.set_value(fraction, window, cx));
                }
                cx.notify();
            }),
            cx.observe(&FavoritesModel::global(cx), |_, _, cx| cx.notify()),
        ];
        let spectrum = cx.new(|cx| Spectrum::new(player.clone(), cx));
        Self {
            player,
            volume,
            progress,
            dragging: None,
            spectrum,
            _subscriptions: subscriptions,
        }
    }

    /// Durée du fichier en cours (pas de déplacement possible dans un direct).
    fn duration(&self, cx: &App) -> Option<Duration> {
        self.player
            .read(cx)
            .track
            .as_ref()
            .filter(|t| !t.live)
            .and_then(|t| t.duration)
            .filter(|d| !d.is_zero())
    }

    fn volume_icon(volume: f32) -> IconName {
        match volume {
            v if v <= 0.0 => IconName::VolumeX,
            v if v < 0.5 => IconName::Volume1,
            _ => IconName::Volume2,
        }
    }
}

/// Sous-titre : état de la lecture, ou erreur.
pub(crate) enum Status {
    Hint(String),
    /// Titre en cours d'une radio, ou artiste et album d'un morceau.
    Info(String),
    Busy(String),
    Error(String),
}

pub(crate) fn status(p: &Player) -> Status {
    if let Some(err) = &p.error {
        return Status::Error(err.clone());
    }
    let format = p
        .track
        .as_ref()
        .and_then(|t| t.format.as_deref())
        .map(|f| format!(" · {}", f.to_uppercase()))
        .unwrap_or_default();
    match p.state {
        PlaybackState::Loading => Status::Busy("Connexion…".into()),
        PlaybackState::Buffering => match p.reconnecting {
            Some(n) => Status::Busy(format!("Reconnexion (tentative {n})…")),
            None => Status::Busy("Mise en tampon…".into()),
        },
        PlaybackState::Playing | PlaybackState::Paused => {
            let paused = if p.state == PlaybackState::Paused {
                "En pause · "
            } else {
                ""
            };
            if let Some(info) = p.track_subtitle() {
                return Status::Info(format!("{paused}{info}"));
            }
            match &p.track {
                Some(t) if t.live => match &p.now_playing {
                    Some(title) => Status::Info(format!("{paused}♪ {title}")),
                    None => {
                        Status::Hint(format!("{paused}En direct · {}{format}", clock(p.position)))
                    }
                },
                Some(_) => Status::Hint(format!("{paused}{}", format.trim_start_matches(" · "))),
                None => Status::Hint(String::new()),
            }
        }
        PlaybackState::Ended => Status::Hint("Terminé".into()),
        PlaybackState::Idle => match (&p.track, &p.resume) {
            (Some(_), _) => Status::Hint("Arrêté".into()),
            (None, Some(_)) => Status::Hint("Dernière écoute · Lecture pour reprendre".into()),
            (None, None) => Status::Hint("Colle une URL de radio ou ouvre un fichier".into()),
        },
    }
}

pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

impl Render for PlayerBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let duration = self.duration(cx);
        let p = self.player.read(cx);

        let title = p.title().unwrap_or_else(|| "Aucune lecture".into());
        let station = p.current_station();
        let cover = p.current_track().and_then(|t| t.cover.clone());
        let favorites = FavoritesModel::global(cx);
        let favorite = station
            .as_ref()
            .is_some_and(|s| favorites.read(cx).contains(&s.url));
        let live = p.track.as_ref().is_some_and(|t| t.live);
        let active = p.state.is_active();
        let has_source = p.has_source();
        let has_queue = !p.queue.is_empty();
        let has_next = p.queue.has_next();
        let shuffle = p.queue.shuffle;
        let repeat = p.repeat();
        let volume = p.volume();
        let position = self.dragging.unwrap_or(p.position);
        let recording = p.recording.is_some();
        let can_record = p.can_record() || recording;

        let subtitle = match status(p) {
            Status::Hint(text) => div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .truncate()
                .child(text),
            Status::Info(text) => div()
                .text_xs()
                .text_color(theme.foreground)
                .opacity(0.8)
                .truncate()
                .child(text),
            Status::Busy(text) => h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(Spinner::new().xsmall())
                .child(text),
            Status::Error(text) => div()
                .text_xs()
                .text_color(theme.danger)
                .truncate()
                .child(text),
        };

        let fallback_color = theme.muted_foreground;
        let artwork = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size_12()
            .rounded(theme.radius)
            .overflow_hidden()
            .bg(theme.muted)
            .text_color(theme.muted_foreground)
            .child(match cover {
                Some(path) => img(path)
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(move || {
                        Icon::new(IconName::Music)
                            .text_color(fallback_color)
                            .into_any_element()
                    })
                    .into_any_element(),
                None => Icon::new(if live {
                    IconName::Radio
                } else {
                    IconName::Music
                })
                .into_any_element(),
            });

        let transport = h_flex()
            .gap_1()
            .items_center()
            .justify_center()
            .child(
                Button::new("shuffle")
                    .icon(IconName::Shuffle)
                    .ghost()
                    .small()
                    .selected(shuffle)
                    .disabled(!has_queue)
                    .tooltip(if shuffle {
                        "Aléatoire : activé"
                    } else {
                        "Aléatoire : désactivé"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.player.update(cx, |p, cx| p.toggle_shuffle(cx));
                    })),
            )
            .child(
                Button::new("prev")
                    .icon(IconName::SkipBack)
                    .ghost()
                    .disabled(!has_queue)
                    .tooltip("Précédent")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.player.update(cx, |p, cx| p.prev(cx));
                    })),
            )
            .child(
                Button::new("stop")
                    .icon(IconName::Square)
                    .ghost()
                    .disabled(!has_source)
                    .tooltip("Arrêter")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.player.update(cx, |p, cx| p.stop(cx));
                    })),
            )
            .child(
                Button::new("play-pause")
                    .icon(if active {
                        IconName::Pause
                    } else {
                        IconName::Play
                    })
                    .primary()
                    .large()
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
                Button::new("next")
                    .icon(IconName::SkipForward)
                    .ghost()
                    .disabled(!has_next)
                    .tooltip("Suivant")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.player.update(cx, |p, cx| p.next(cx));
                    })),
            )
            .child(
                Button::new("repeat")
                    .icon(if repeat == Repeat::One {
                        IconName::Repeat1
                    } else {
                        IconName::Repeat
                    })
                    .ghost()
                    .small()
                    .selected(repeat != Repeat::Off)
                    .disabled(!has_queue)
                    .tooltip(match repeat {
                        Repeat::Off => "Répéter : non",
                        Repeat::All => "Répéter : toute la file",
                        Repeat::One => "Répéter : ce morceau",
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.player.update(cx, |p, cx| p.cycle_repeat(cx));
                    })),
            );

        let timeline = duration.map(|duration| {
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(div().w(px(40.)).text_right().child(clock(position)))
                .child(
                    div()
                        .flex_1()
                        .child(Slider::new(&self.progress).accessibility_label("Position")),
                )
                .child(div().w(px(40.)).child(clock(duration)))
        });

        h_flex()
            .w_full()
            .h(px(80.))
            .flex_shrink_0()
            .px_4()
            .gap_4()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            // Pochette + titre
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_3()
                    .items_center()
                    .child(artwork)
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_sm().font_medium().truncate().child(title))
                            .child(subtitle),
                    )
                    .when_some(station, |row, station| {
                        let favs = favorites.clone();
                        row.child(
                            Button::new("player-fav")
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
                                    toggle_favorite(&favs, &station, window, cx);
                                }),
                        )
                    })
                    .when(can_record, |row| {
                        row.child(
                            Button::new("record")
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
                    }),
            )
            // Transport + progression
            .child(
                v_flex()
                    .w(px(420.))
                    .flex_shrink_0()
                    .gap_1()
                    .child(transport)
                    .children(timeline),
            )
            // Volume
            .child(
                h_flex()
                    .flex_1()
                    .justify_end()
                    .gap_2()
                    .items_center()
                    .child(div().mr_2().child(self.spectrum.clone()))
                    .child(
                        Button::new("visualizer")
                            .icon(IconName::Sparkles)
                            .ghost()
                            .small()
                            .tooltip("Visualiseur (Ctrl+Maj+V)")
                            .on_click(|_, window, cx| {
                                window
                                    .dispatch_action(Box::new(crate::actions::ToggleVisualizer), cx)
                            }),
                    )
                    .child(
                        Button::new("compact")
                            .icon(IconName::PictureInPicture2)
                            .ghost()
                            .small()
                            .tooltip("Vue compacte (Ctrl+Maj+M)")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(crate::actions::ToggleCompact), cx)
                            }),
                    )
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(Self::volume_icon(volume)),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .child(Slider::new(&self.volume).accessibility_label("Volume")),
                    )
                    .child(
                        div()
                            .w(px(36.))
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{:.0}%", volume * 100.)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::clock;
    use std::time::Duration;

    #[test]
    fn formats_clock() {
        assert_eq!(clock(Duration::from_secs(5)), "0:05");
        assert_eq!(clock(Duration::from_secs(754)), "12:34");
        assert_eq!(clock(Duration::from_secs(3_725)), "1:02:05");
    }
}
