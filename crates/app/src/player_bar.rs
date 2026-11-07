//! Barre de lecture (interface seule en phase 0 ; branchée au moteur en phase 1).

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    slider::{Slider, SliderEvent, SliderState},
    v_flex,
};
use gpui_kit::*;

pub struct PlayerBar {
    volume: Entity<SliderState>,
    volume_value: f32,
    playing: bool,
    _subscriptions: Vec<Subscription>,
}

impl PlayerBar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let volume_value = 0.8;
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.01)
                .default_value(volume_value)
        });
        let sub = cx.subscribe(&volume, |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.volume_value = value.start();
                cx.notify();
            }
        });
        Self {
            volume,
            volume_value,
            playing: false,
            _subscriptions: vec![sub],
        }
    }

    fn volume_icon(&self) -> IconName {
        match self.volume_value {
            v if v <= 0.0 => IconName::VolumeX,
            v if v < 0.5 => IconName::Volume1,
            _ => IconName::Volume2,
        }
    }
}

impl Render for PlayerBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

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
                            .child(IconName::Music),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_sm().font_medium().child("Aucune lecture"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Choisis une radio ou un fichier"),
                            ),
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
                            .tooltip("Précédent"),
                    )
                    .child(
                        Button::new("play-pause")
                            .icon(if self.playing {
                                IconName::Pause
                            } else {
                                IconName::Play
                            })
                            .primary()
                            .large()
                            .tooltip(if self.playing { "Pause" } else { "Lecture" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.playing = !this.playing;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("next")
                            .icon(IconName::SkipForward)
                            .ghost()
                            .tooltip("Suivant"),
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
                            .child(self.volume_icon()),
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
                            .child(format!("{:.0}%", self.volume_value * 100.)),
                    ),
            )
    }
}
