//! Spectre de ce qui est joué, dans la barre de lecture.

use gpui_kit::component::{ActiveTheme, h_flex};
use gpui_kit::*;
use ondes_audio::PlaybackState;

use crate::player::Player;
use crate::settings::Settings;

/// Descente maximale d'une barre par image (retombée progressive).
const FALL: f32 = 0.07;
/// Cadence d'animation : 30 images/s suffisent, et redessiner à chaque
/// image d'un écran à 144 Hz coûte cher.
const FRAME: std::time::Duration = std::time::Duration::from_millis(33);

pub struct Spectrum {
    player: Entity<Player>,
    levels: Vec<f32>,
    raw: Vec<f32>,
    height: f32,
    bar_width: f32,
    /// Affiché même si le spectre est masqué dans les réglages (vue compacte).
    always: bool,
    /// Minuterie d'animation, active pendant la lecture et la retombée.
    tick: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Spectrum {
    /// Petit spectre de la barre de lecture.
    pub fn new(player: Entity<Player>, cx: &mut Context<Self>) -> Self {
        Self::with_size(player, 20, 28., 3., false, cx)
    }

    pub fn with_size(
        player: Entity<Player>,
        bars: usize,
        height: f32,
        bar_width: f32,
        always: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&player, |_, _, cx| cx.notify()),
            cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()),
        ];
        Self {
            player,
            levels: vec![0.; bars],
            raw: vec![0.; bars],
            height,
            bar_width,
            always,
            tick: None,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for Spectrum {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.always || Settings::global(cx).read(cx).state.audio.visualizer;
        let p = self.player.read(cx);
        let Some(analyzer) = p.analyzer().filter(|_| enabled) else {
            return div().into_any_element();
        };
        self.raw.iter_mut().for_each(|r| *r = 0.);
        if p.state == PlaybackState::Playing {
            analyzer.spectrum(&mut self.raw);
        }
        let mut moving = false;
        for (level, &target) in self.levels.iter_mut().zip(&self.raw) {
            let next = target.max(*level - FALL);
            moving |= next > 0.001 || (next - *level).abs() > 0.001;
            *level = next.max(0.);
        }
        // Animé tant que la musique joue ou que les barres retombent.
        let animate = moving || p.state == PlaybackState::Playing;
        if !animate {
            self.tick = None;
        } else if self.tick.is_none() {
            self.tick = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(FRAME).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        }

        let color = cx.theme().primary;
        let (height, width) = (self.height, self.bar_width);
        h_flex()
            .h(px(height))
            .gap(px((width * 0.6).max(2.)))
            .items_end()
            .flex_shrink_0()
            .children(self.levels.iter().map(|&level| {
                div()
                    .w(px(width))
                    .h(px((level * height).max(1.)))
                    .rounded_t(px(width / 3.))
                    .bg(color.opacity(0.35 + 0.65 * level))
            }))
            .into_any_element()
    }
}
