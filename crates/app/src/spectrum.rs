//! Spectre de ce qui est joué, dans la barre de lecture.

use gpui_kit::component::{ActiveTheme, h_flex};
use gpui_kit::*;
use ondes_audio::PlaybackState;

use crate::player::Player;
use crate::settings::Settings;

const BARS: usize = 20;
const HEIGHT: f32 = 28.;
/// Descente maximale d'une barre par image (retombée progressive).
const FALL: f32 = 0.035;

pub struct Spectrum {
    player: Entity<Player>,
    levels: [f32; BARS],
    _subscriptions: Vec<Subscription>,
}

impl Spectrum {
    pub fn new(player: Entity<Player>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&player, |_, _, cx| cx.notify()),
            cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()),
        ];
        Self {
            player,
            levels: [0.; BARS],
            _subscriptions: subscriptions,
        }
    }
}

impl Render for Spectrum {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = Settings::global(cx).read(cx).state.audio.visualizer;
        let p = self.player.read(cx);
        let Some(analyzer) = p.analyzer().filter(|_| enabled) else {
            return div().into_any_element();
        };
        let mut raw = [0f32; BARS];
        if p.state == PlaybackState::Playing {
            analyzer.spectrum(&mut raw);
        }
        let mut moving = false;
        for (level, &target) in self.levels.iter_mut().zip(&raw) {
            let next = target.max(*level - FALL);
            moving |= next > 0.001 || (next - *level).abs() > 0.001;
            *level = next.max(0.);
        }
        // Animé tant que la musique joue ou que les barres retombent.
        if moving || p.state == PlaybackState::Playing {
            window.request_animation_frame();
        }

        let color = cx.theme().primary;
        h_flex()
            .h(px(HEIGHT))
            .gap(px(2.))
            .items_end()
            .flex_shrink_0()
            .children(self.levels.iter().map(|&level| {
                div()
                    .w(px(3.))
                    .h(px((level * HEIGHT).max(1.)))
                    .rounded_t(px(1.))
                    .bg(color.opacity(0.35 + 0.65 * level))
            }))
            .into_any_element()
    }
}
