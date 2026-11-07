//! Barre de lecture, reliée au modèle [`Player`].

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    slider::{Slider, SliderEvent, SliderState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::*;
use ondes_audio::PlaybackState;

use crate::player::Player;

pub struct PlayerBar {
    player: Entity<Player>,
    volume: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
}

impl PlayerBar {
    pub fn new(player: Entity<Player>, cx: &mut Context<Self>) -> Self {
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
            cx.observe(&player, |_, _, cx| cx.notify()),
        ];
        Self {
            player,
            volume,
            _subscriptions: subscriptions,
        }
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
enum Status {
    Hint(String),
    Busy(String),
    Error(String),
}

fn status(p: &Player) -> Status {
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
            match &p.track {
                Some(t) if t.live => {
                    Status::Hint(format!("{paused}En direct · {}{format}", clock(p.position)))
                }
                Some(t) => Status::Hint(format!(
                    "{paused}{} / {}{format}",
                    clock(p.position),
                    t.duration.map(clock).unwrap_or_else(|| "--:--".into())
                )),
                None => Status::Hint(String::new()),
            }
        }
        PlaybackState::Ended => Status::Hint("Terminé".into()),
        PlaybackState::Idle => match &p.track {
            Some(_) => Status::Hint("Arrêté".into()),
            None => Status::Hint("Colle une URL de radio ou ouvre un fichier".into()),
        },
    }
}

fn clock(d: Duration) -> String {
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
        let p = self.player.read(cx);

        let title = p
            .track
            .as_ref()
            .map(|t| t.title.clone())
            .or_else(|| p.pending.clone())
            .unwrap_or_else(|| "Aucune lecture".into());
        let live = p.track.as_ref().is_some_and(|t| t.live);
        let active = p.state.is_active();
        let has_source = p.has_source();
        let volume = p.volume();

        let subtitle = match status(p) {
            Status::Hint(text) => div()
                .text_xs()
                .text_color(theme.muted_foreground)
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

        h_flex()
            .w_full()
            .h(px(72.))
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
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .size_12()
                            .rounded(theme.radius)
                            .bg(theme.muted)
                            .text_color(theme.muted_foreground)
                            .child(if live {
                                IconName::Radio
                            } else {
                                IconName::Music
                            }),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_sm().font_medium().truncate().child(title))
                            .child(subtitle),
                    ),
            )
            // Transport
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("prev")
                            .icon(IconName::SkipBack)
                            .ghost()
                            .disabled(true)
                            .tooltip("Précédent (file d'attente : phase 3)"),
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
                            .disabled(true)
                            .tooltip("Suivant (file d'attente : phase 3)"),
                    ),
            )
            // Volume
            .child(
                h_flex()
                    .flex_1()
                    .justify_end()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(Self::volume_icon(volume)),
                    )
                    .child(
                        div()
                            .w(px(120.))
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
